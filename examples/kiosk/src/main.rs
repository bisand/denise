//! M2 end to end: a real display, real input, and an event loop that sleeps.
//!
//! Everything M2 promised, wired together — DRM/KMS scanout, evdev input, damage
//! tracking, the theme — on a machine with no desktop environment.
//!
//! The loop is the point. It blocks in the kernel on the input descriptors until
//! either something happens or the next frame is due, and when nothing is moving
//! it does not wake at all. A UI that costs nothing while idle is the entire
//! premise of the project, and a loop that spins would quietly give it away.
//!
//! ```text
//! /tmp/kiosk [seconds]
//! ```
//!
//! Move the pointer, click, press `T` for the next theme, `Escape` or `Q` to quit.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("the kiosk demo needs Linux, DRM and evdev");
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::fd::BorrowedFd;
    use std::time::{Duration, Instant};

    use denise::{
        Color, DamageTracker, ElementState, InputEvent, InputSource, KeyCode, MAX_DAMAGE_RECTS,
        Point, Radius, Rect, Role, Size, Surface, Theme,
    };
    use denise_drm::{DrmSurface, SurfaceConfig};
    use denise_evdev::InputBackend;
    use denise_fbdev::FbdevSurface;
    use denise_render::Canvas;
    use rustix::event::{PollFd, PollFlags, Timespec, poll};

    const CURSOR: i32 = 28;

    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(60)
        .clamp(1, 600);

    // The shortest gap allowed between presents — a runaway guard, not a schedule.
    //
    // This defaulted to 60 Hz, which measured badly on real hardware. A Logitech
    // K400 reports every 16 ms, so a 60 Hz cap runs at almost exactly the input
    // rate: the two drift in and out of phase and an event lands at a uniformly
    // random point in the frame window, waiting up to a full frame for nothing.
    // Measured on a Pi 3 A+, that was 6.8 ms of median latency and 15.5 ms at p95,
    // and it was visible as drag.
    //
    // Raising it does not add frames, because frames here are bound by the input
    // rate rather than the cap: at 250 Hz the observed interval stayed at 16 ms,
    // one present per event, and median latency fell to 0.16 ms. So the cap is set
    // far above any input rate and does nothing until something goes wrong.
    //
    // The caveat is animation. This scene only changes when input arrives, so
    // input-bound and damage-bound are the same thing. A scene that animates on its
    // own damages something every iteration and would run at the cap, so it needs
    // to pace itself rather than lean on this.
    let hz: u64 = std::env::args()
        .nth(2)
        .and_then(|a| a.parse().ok())
        .unwrap_or(250)
        .clamp(10, 1000);
    let frame = Duration::from_micros(1_000_000 / hz);

    // DRM first, because it is the one that page-flips and knows about vblank.
    // fbdev is not a lesser configuration of the same thing: on a Pi with no
    // vc4-kms-v3d overlay it is the only display there is.
    let mut surface: Box<dyn Surface> = match DrmSurface::open(SurfaceConfig::default()) {
        Ok(drm) => {
            eprintln!(
                "display DRM/KMS {} — {} buffers",
                drm.mode_name(),
                drm.buffer_count()
            );
            Box::new(drm)
        }
        Err(drm_error) => match FbdevSurface::open_first() {
            Ok(fb) => {
                eprintln!("display fbdev {} ({})", fb.info(), fb.path().display());
                eprintln!("        no DRM: {drm_error}");
                eprintln!("        no page flip and no vsync, so this can tear");
                Box::new(fb)
            }
            Err(fb_error) => {
                return Err(format!("no display — DRM: {drm_error}; fbdev: {fb_error}").into());
            }
        },
    };

    let size = surface.size();
    let mut input = InputBackend::open_all(size)?;

    for device in input.devices() {
        eprintln!("input   {}: {}", device.capabilities(), device.name());
    }
    eprintln!(
        "present no more often than every {:.1} ms ({hz} Hz cap)",
        frame.as_secs_f64() * 1000.0
    );
    eprintln!("\npointer to move, click, T for theme, Escape or Q to quit\n");

    // Built once: the device set does not change, and `poll` updates each entry's
    // revents in place.
    let raw_fds = input.raw_fds();
    let borrowed: Vec<BorrowedFd<'_>> = raw_fds
        .iter()
        // SAFETY: every descriptor belongs to `input`, which outlives this loop and
        // holds each device open until the process exits.
        .map(|&fd| unsafe { BorrowedFd::borrow_raw(fd) })
        .collect();
    let mut poll_fds: Vec<PollFd<'_>> = borrowed
        .iter()
        .map(|fd| PollFd::new(fd, PollFlags::IN))
        .collect();

    let mut themes = Theme::BUILT_IN.iter().cycle();
    let mut active: Theme = *themes.next().expect("built-in themes are not empty");

    let mut tracker = DamageTracker::new(size);
    let mut cursor = input.pointer();
    let mut card = Rect::new(
        size.width as i32 / 4,
        size.height as i32 / 3,
        (size.width as i32 / 3).max(160),
        (size.height as i32 / 4).max(120),
    );
    let mut held = false;
    let mut clicks = 0u32;

    let cursor_rect = |p: Point| Rect::new(p.x - CURSOR / 2, p.y - CURSOR / 2, CURSOR, CURSOR);

    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut next_frame = Instant::now();
    let mut events = Vec::new();
    let mut frames = 0u64;
    let mut woke = 0u64;
    let mut quit = false;
    let started = Instant::now();

    // Latency accounting. "Drag" can mean two very different things — the frame
    // arriving late, or the pointer travelling too little per unit of hand
    // movement — and they have opposite fixes, so measure rather than guess.
    // What is actually on screen. Input updates the live state; these lag behind
    // until a frame is presented, and the difference between them is exactly the
    // damage. Deriving damage this way rather than per event is what makes
    // coalescing correct: ten moves folded into one frame damage two rectangles,
    // not twenty, because the intermediate positions were never drawn and so have
    // nothing to erase.
    //
    // The snapshot has to cover everything that changes the pixels, not just
    // position. `held` decides the card's colour, so a press or release with no
    // movement still has to be damage. Leaving it out looked like the pointer
    // wiping a trail across the card: the card kept its stale colour, and only the
    // patches repainted under the moving cursor showed the right one.
    let mut drawn_cursor = cursor;
    let mut drawn_card = card;
    let mut drawn_held = held;

    // Is damage growing when frames get slow? That is the feedback loop to look
    // for: a slow frame lets the pointer travel further, which enlarges the next
    // damage region, which makes that frame slower again.
    // Age of the event when we read it: queuing the loop cannot otherwise see.
    let mut queued_us: Vec<u32> = Vec::new();
    let mut damage_px: Vec<u32> = Vec::new();
    let mut events_per_poll: Vec<u32> = Vec::new();
    let mut slow_frames = 0u64;

    let mut render_us: Vec<u32> = Vec::new();
    let mut latency_us: Vec<u32> = Vec::new();
    let mut interval_us: Vec<u32> = Vec::new();
    let mut motion_events = 0u64;
    let mut motion_pixels = 0i64;
    // When the input that this frame will show first arrived.
    let mut dirty_since: Option<Instant> = None;
    let mut last_present: Option<Instant> = None;

    while !quit && Instant::now() < deadline {
        // Sleep until input arrives or the next frame is due. With nothing moving
        // there is no next frame, so this blocks outright and the process uses no
        // CPU at all until a finger lands on the panel.
        let timeout = if tracker.is_clean() {
            Duration::from_millis(250)
        } else {
            next_frame.saturating_duration_since(Instant::now())
        };
        let spec = Timespec {
            tv_sec: timeout.as_secs() as _,
            tv_nsec: timeout.subsec_nanos() as _,
        };
        let _ = poll(&mut poll_fds, Some(&spec));
        woke += 1;

        events.clear();
        input.poll(&mut events);
        if !events.is_empty() {
            events_per_poll.push(events.len() as u32);
            if let Some(age) = input.last_event_age() {
                queued_us.push(age.as_micros().min(u128::from(u32::MAX)) as u32);
            }
        }
        if !events.is_empty() && dirty_since.is_none() {
            dirty_since = Some(Instant::now());
        }

        for event in &events {
            match event {
                InputEvent::PointerMoved { position } => {
                    motion_events += 1;
                    motion_pixels += i64::from((position.x - cursor.x).abs())
                        + i64::from((position.y - cursor.y).abs());
                    cursor = *position;
                }
                InputEvent::PointerButton {
                    button,
                    state,
                    position,
                    ..
                } => {
                    eprintln!("  {button:?} {state:?} at {},{}", position.x, position.y);
                    held = state.is_down();
                    if held {
                        clicks += 1;
                    }
                }
                InputEvent::TouchDown { position, .. }
                | InputEvent::TouchMoved { position, .. } => {
                    cursor = *position;
                    held = true;
                }
                InputEvent::TouchUp { .. } => {
                    held = false;
                }
                InputEvent::Key {
                    code,
                    state: ElementState::Down,
                    repeat: false,
                    modifiers,
                    ..
                } => {
                    // Echoed so the key path is visible while testing. These are
                    // positions, not characters: on a Norwegian layout the key
                    // carrying o-slash reports as Semicolon, which is correct.
                    eprintln!("  key {code:?}{}", modifier_suffix(*modifiers));
                    match code {
                        KeyCode::Escape | KeyCode::Q => quit = true,
                        KeyCode::T => {
                            active = *themes.next().expect("cycle never ends");
                            // Everything is themed, so everything is dirty.
                            tracker.add_full();
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        // The card follows the pointer while a button or finger is down, which is
        // the whole point: input reaching the display through the same damage path
        // as everything else.
        if held {
            let next = Rect::new(
                (cursor.x - card.width / 2).clamp(0, size.width as i32 - card.width),
                (cursor.y - card.height / 2).clamp(0, size.height as i32 - card.height),
                card.width,
                card.height,
            );
            card = next;
        }

        // One diff against the screen, however many events were folded in.
        if cursor != drawn_cursor {
            tracker.add(cursor_rect(drawn_cursor));
            tracker.add(cursor_rect(cursor));
        }
        if card != drawn_card || held != drawn_held {
            tracker.add(drawn_card);
            tracker.add(card);
        }

        if tracker.is_clean() {
            continue;
        }
        if Instant::now() < next_frame {
            continue;
        }
        next_frame = Instant::now() + frame;
        let render_start = Instant::now();

        let mut frame = surface.acquire()?;
        let mut regions = [Rect::ZERO; MAX_DAMAGE_RECTS];
        let count = {
            let resolved = tracker.resolve(frame.age());
            regions[..resolved.len()].copy_from_slice(resolved);
            resolved.len()
        };
        let damage = &regions[..count];
        damage_px.push(damage.iter().map(Rect::area).sum::<u64>() as u32);

        {
            let mut canvas = Canvas::new(&mut frame);
            for region in damage {
                let mut c = canvas.with_clip(*region);
                paint(&mut c, active, card, cursor_rect(cursor), held, size);
            }
        }

        drop(frame);
        surface.present(damage)?;
        tracker.end_frame();
        drawn_cursor = cursor;
        drawn_card = card;
        drawn_held = held;
        frames += 1;

        let now = Instant::now();
        let took = now.duration_since(render_start);
        if took > Duration::from_millis(16) {
            slow_frames += 1;
        }
        render_us.push(took.as_micros() as u32);
        if let Some(since) = dirty_since.take() {
            latency_us.push(now.duration_since(since).as_micros() as u32);
        }
        if let Some(previous) = last_present.replace(now) {
            interval_us.push(now.duration_since(previous).as_micros() as u32);
        }
    }

    let elapsed = started.elapsed().as_secs_f64();
    eprintln!(
        "\n{frames} frames drawn over {elapsed:.1}s ({:.1}/s), {woke} loop wake-ups, {clicks} clicks",
        frames as f64 / elapsed
    );
    eprintln!("frames only cost anything when something changed — that is the whole idea");

    eprintln!(
        "{slow_frames} frames took longer than 16 ms ({:.1}% of {frames})",
        slow_frames as f64 / frames.max(1) as f64 * 100.0
    );
    report("queued     hardware..read  ", &mut queued_us);
    report_raw("damage     pixels per frame", &mut damage_px, 1.0);
    report_raw("backlog    events per poll ", &mut events_per_poll, 1.0);
    report("render     acquire..present", &mut render_us);
    report("latency    input..present  ", &mut latency_us);
    report("interval   present..present", &mut interval_us);

    if motion_events > 0 {
        eprintln!(
            "\n{motion_events} pointer events moved {motion_pixels} px in total \
             = {:.2} px per event",
            motion_pixels as f64 / motion_events as f64
        );
        eprintln!(
            "device reported roughly {:.0} events/s, {:.2} motion events per frame",
            motion_events as f64 / elapsed,
            motion_events as f64 / frames.max(1) as f64
        );
    }

    /// Prints the shape of a sample set. Percentiles, not a mean: a mean hides
    /// exactly the occasional long frame that gets noticed as a stutter.
    fn report(label: &str, samples: &mut [u32]) {
        if samples.is_empty() {
            return;
        }
        samples.sort_unstable();
        let at = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize] as f64 / 1000.0;
        eprintln!(
            "{label}  n={:<5} p50 {:>8.2} ms   p95 {:>8.2} ms   max {:>8.2} ms",
            samples.len(),
            at(0.50),
            at(0.95),
            at(1.0)
        );
    }

    /// As [`report`], for samples that are not durations.
    fn report_raw(label: &str, samples: &mut [u32], scale: f64) {
        if samples.is_empty() {
            return;
        }
        samples.sort_unstable();
        let at = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize] as f64 * scale;
        eprintln!(
            "{label}  n={:<5} p50 {:>8.0}      p95 {:>8.0}      max {:>8.0}",
            samples.len(),
            at(0.50),
            at(0.95),
            at(1.0)
        );
    }

    /// Renders held modifiers, or nothing when none are.
    fn modifier_suffix(modifiers: denise::Modifiers) -> String {
        if modifiers.is_empty() {
            return String::new();
        }
        let mut held = Vec::new();
        for (bit, name) in [
            (denise::Modifiers::CTRL, "ctrl"),
            (denise::Modifiers::ALT, "alt"),
            (denise::Modifiers::SHIFT, "shift"),
            (denise::Modifiers::SUPER, "super"),
        ] {
            if modifiers.contains(bit) {
                held.push(name);
            }
        }
        format!("  [{}]", held.join("+"))
    }

    /// Paints the scene. Unaware of damage: the clip is what makes it incremental.
    fn paint(
        canvas: &mut Canvas<'_>,
        theme: Theme,
        card: Rect,
        cursor: Rect,
        held: bool,
        size: Size,
    ) {
        canvas.clear(theme.color(Role::Base100));

        // A header band, so a theme change is obvious at a glance.
        canvas.fill_rect(
            Rect::new(0, 0, size.width as i32, 64),
            theme.color(Role::Base200),
        );
        canvas.fill_rect(
            Rect::new(0, 64, size.width as i32, 2),
            theme.color(Role::Base300),
        );

        let surface_role = if held { Role::Accent } else { Role::Primary };
        canvas.fill_rounded_rect(card, theme.radius(Radius::Box), theme.color(surface_role));
        canvas.stroke_rounded_rect(
            card,
            theme.radius(Radius::Box),
            2,
            theme.color(Role::Base300),
        );

        // A band in the paired content colour, which is guaranteed to be legible
        // against whatever the surface role resolved to.
        canvas.fill_rect(
            Rect::new(card.x + 20, card.y + 20, card.width - 40, 26),
            theme.content_of(surface_role),
        );

        // The cursor sprite, composited last, exactly as the pipeline says.
        canvas.fill_rounded_rect(
            cursor,
            theme.radius(Radius::Selector),
            Color::rgba(255, 255, 255, 220),
        );
        canvas.stroke_rounded_rect(
            cursor,
            theme.radius(Radius::Selector),
            2,
            theme.color(Role::BaseContent),
        );
    }

    Ok(())
}
