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

    /// Frame budget. The display should pace us, but a driver that retires flips
    /// immediately — every virtualised GPU — would otherwise let this run flat out.
    const FRAME: Duration = Duration::from_millis(16);
    const CURSOR: i32 = 28;

    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(60)
        .clamp(1, 600);

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

        for event in &events {
            match event {
                InputEvent::PointerMoved { position } => {
                    tracker.add(cursor_rect(cursor));
                    cursor = *position;
                    tracker.add(cursor_rect(cursor));
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
                    tracker.add(card);
                }
                InputEvent::TouchDown { position, .. }
                | InputEvent::TouchMoved { position, .. } => {
                    tracker.add(cursor_rect(cursor));
                    cursor = *position;
                    held = true;
                    tracker.add(cursor_rect(cursor));
                }
                InputEvent::TouchUp { .. } => {
                    held = false;
                    tracker.add(card);
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
            if next != card {
                tracker.add(card);
                card = next;
                tracker.add(card);
            }
        }

        if tracker.is_clean() {
            continue;
        }
        if Instant::now() < next_frame {
            continue;
        }
        next_frame = Instant::now() + FRAME;

        let mut frame = surface.acquire()?;
        let mut regions = [Rect::ZERO; MAX_DAMAGE_RECTS];
        let count = {
            let resolved = tracker.resolve(frame.age());
            regions[..resolved.len()].copy_from_slice(resolved);
            resolved.len()
        };
        let damage = &regions[..count];

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
        frames += 1;
    }

    let elapsed = started.elapsed().as_secs_f64();
    eprintln!(
        "\n{frames} frames drawn over {elapsed:.1}s ({:.1}/s), {woke} loop wake-ups, {clicks} clicks",
        frames as f64 / elapsed
    );
    eprintln!("frames only cost anything when something changed — that is the whole idea");

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
