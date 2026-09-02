//! M2 end to end: a real display, real input, and an event loop that sleeps.
//!
//! Everything M2 promised, wired together — DRM/KMS scanout, evdev input, damage
//! tracking, the theme — on a machine with no desktop environment.
//!
//! ```text
//! /tmp/kiosk [seconds] [frame-cap-hz] [vsync]
//! ```
//!
//! Move the pointer, click, press `T` for the next theme, `Escape` or `Q` to quit.
//!
//! # Where the input is read matters
//!
//! On a backend that page-flips, `acquire` blocks until the display retires the
//! previous flip. Reading input before that wait means drawing a position already
//! up to a refresh period old — measured on a Pi at 6.2 ms of queuing at p50 and
//! 15 ms at p95, on top of the flip's own latency, and plainly visible as drag. So
//! the wait comes first and input is read immediately after it, as late as
//! possible before rasterising.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("the kiosk demo needs Linux, DRM and evdev");
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run()
}

#[cfg(target_os = "linux")]
mod app {
    use std::os::fd::BorrowedFd;
    use std::time::{Duration, Instant};

    use denise::{
        Color, DamageTracker, ElementState, InputEvent, InputSource, KeyCode, MAX_DAMAGE_RECTS,
        Modifiers, Point, Radius, Rect, Role, Size, Surface, Theme,
    };
    use denise_drm::{DrmSurface, PresentMode, SurfaceConfig};
    use denise_evdev::InputBackend;
    use denise_fbdev::FbdevSurface;
    use denise_render::{Canvas, Pen, Pen};
    use rustix::event::{PollFd, PollFlags, Timespec, poll};

    const CURSOR: i32 = 28;

    /// Everything input can change, and what of it is currently on screen.
    ///
    /// The `drawn_*` fields are the snapshot: damage is one diff between them and
    /// the live state, taken once per frame. That is what makes coalescing
    /// correct — a hundred pointer events folded into one frame damage two
    /// rectangles, not two hundred, because the intermediate positions were never
    /// painted and so have nothing to erase.
    ///
    /// The snapshot must hold everything that decides the pixels, not just
    /// geometry. `held` picks the card's colour, so a press with no movement is
    /// still damage. Leaving it out looked like the pointer wiping a trail across
    /// the card: only the patches repainted under the cursor got the new colour.
    struct Scene {
        size: Size,
        cursor: Point,
        card: Rect,
        held: bool,

        drawn_cursor: Point,
        drawn_card: Rect,
        drawn_held: bool,

        theme: usize,
        quit: bool,
        clicks: u32,
        motion_events: u64,
        motion_pixels: i64,
    }

    impl Scene {
        fn new(size: Size, cursor: Point) -> Self {
            let card = Rect::new(
                size.width as i32 / 4,
                size.height as i32 / 3,
                (size.width as i32 / 3).max(160),
                (size.height as i32 / 4).max(120),
            );
            Self {
                size,
                cursor,
                card,
                held: false,
                drawn_cursor: cursor,
                drawn_card: card,
                drawn_held: false,
                theme: 0,
                quit: false,
                clicks: 0,
                motion_events: 0,
                motion_pixels: 0,
            }
        }

        fn theme(&self) -> Theme {
            Theme::BUILT_IN[self.theme % Theme::BUILT_IN.len()]
        }

        fn cursor_rect(&self, at: Point) -> Rect {
            Rect::new(at.x - CURSOR / 2, at.y - CURSOR / 2, CURSOR, CURSOR)
        }

        /// Folds a batch of events into the live state. Draws nothing and records
        /// no damage; that is derived once, later, from the snapshot.
        fn apply(&mut self, events: &[InputEvent], tracker: &mut DamageTracker) {
            for event in events {
                match event {
                    InputEvent::PointerMoved { position } => {
                        self.motion_events += 1;
                        self.motion_pixels += i64::from((position.x - self.cursor.x).abs())
                            + i64::from((position.y - self.cursor.y).abs());
                        self.cursor = *position;
                    }
                    InputEvent::PointerButton {
                        button,
                        state,
                        position,
                        ..
                    } => {
                        eprintln!("  {button:?} {state:?} at {},{}", position.x, position.y);
                        self.held = state.is_down();
                        if self.held {
                            self.clicks += 1;
                        }
                    }
                    InputEvent::TouchDown { position, .. }
                    | InputEvent::TouchMoved { position, .. } => {
                        self.cursor = *position;
                        self.held = true;
                    }
                    InputEvent::TouchUp { .. } => self.held = false,
                    InputEvent::Key {
                        code,
                        state: ElementState::Down,
                        repeat: false,
                        modifiers,
                        ..
                    } => {
                        eprintln!("  key {code:?}{}", modifier_suffix(*modifiers));
                        match code {
                            KeyCode::Escape | KeyCode::Q => self.quit = true,
                            KeyCode::T => {
                                self.theme += 1;
                                // Everything is themed, so everything is dirty.
                                tracker.add_full();
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }

            // The card follows the pointer while a button or finger is down.
            if self.held {
                self.card = Rect::new(
                    (self.cursor.x - self.card.width / 2)
                        .clamp(0, self.size.width as i32 - self.card.width),
                    (self.cursor.y - self.card.height / 2)
                        .clamp(0, self.size.height as i32 - self.card.height),
                    self.card.width,
                    self.card.height,
                );
            }
        }

        /// One diff against the screen, however many events were folded in.
        fn damage(&self, tracker: &mut DamageTracker) {
            if self.cursor != self.drawn_cursor {
                tracker.add(self.cursor_rect(self.drawn_cursor));
                tracker.add(self.cursor_rect(self.cursor));
            }
            if self.card != self.drawn_card || self.held != self.drawn_held {
                tracker.add(self.drawn_card);
                tracker.add(self.card);
            }
        }

        fn presented(&mut self) {
            self.drawn_cursor = self.cursor;
            self.drawn_card = self.card;
            self.drawn_held = self.held;
        }

        /// Paints the scene. Unaware of damage: the clip makes it incremental.
        fn paint(&self, canvas: &mut Pen<'_>) {
            let theme = self.theme();
            canvas.clear(theme.color(Role::Base100));

            // A header band, so a theme change is obvious at a glance.
            canvas.fill_rect(
                Rect::new(0, 0, self.size.width as i32, 64),
                theme.color(Role::Base200),
            );
            canvas.fill_rect(
                Rect::new(0, 64, self.size.width as i32, 2),
                theme.color(Role::Base300),
            );

            let surface_role = if self.held {
                Role::Accent
            } else {
                Role::Primary
            };
            canvas.fill_rounded_rect(
                self.card,
                theme.radius(Radius::Box),
                theme.color(surface_role),
            );
            canvas.stroke_rounded_rect(
                self.card,
                theme.radius(Radius::Box),
                2,
                theme.color(Role::Base300),
            );

            // A band in the paired content colour, guaranteed legible against
            // whatever the surface role resolved to.
            canvas.fill_rect(
                Rect::new(self.card.x + 20, self.card.y + 20, self.card.width - 40, 26),
                theme.content_of(surface_role),
            );

            // The cursor sprite, composited last, exactly as the pipeline says.
            let cursor = self.cursor_rect(self.cursor);
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
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let seconds: u64 = std::env::args()
            .nth(1)
            .and_then(|a| a.parse().ok())
            .unwrap_or(60)
            .clamp(1, 600);

        // A runaway guard, not a schedule. It sits far above any input rate and
        // does nothing until something goes wrong: a driver that retires flips
        // instantly, as every virtualised GPU does, would otherwise let this run
        // flat out. A cap near the input rate is actively harmful — at 60 Hz
        // against a mouse reporting every 16 ms the two beat, and every event
        // waits a random slice of a frame for no gain at all.
        let hz: u64 = std::env::args()
            .nth(2)
            .and_then(|a| a.parse().ok())
            .unwrap_or(250)
            .clamp(10, 1000);
        let frame = Duration::from_micros(1_000_000 / hz);

        // "immediate" asks for async page flips: no wait for vblank, at the cost
        // of a tear. Which way this should go depends on the product — but the
        // default is measured rather than assumed: on a Pi 3 the vc4's async
        // flips tear visibly enough to read as flicker, and a panel is read
        // rather than aimed at, so 17 ms is invisible and a tear is not.
        let requested = match std::env::args().nth(3).as_deref() {
            Some("immediate") | Some("tearing") => PresentMode::Immediate,
            _ => PresentMode::Vsync,
        };

        // DRM first, because it page-flips and knows about vblank. fbdev is not a
        // lesser configuration of the same thing: on a Pi with no vc4-kms-v3d
        // overlay it is the only display there is.
        let config = SurfaceConfig {
            present_mode: requested,
            ..SurfaceConfig::default()
        };
        let mut surface: Box<dyn Surface> = match DrmSurface::open(config) {
            Ok(drm) => {
                let actual = drm.present_mode();
                eprintln!(
                    "display DRM/KMS {} — {} buffers, {}",
                    drm.mode_name(),
                    drm.buffer_count(),
                    match actual {
                        PresentMode::Vsync => "vsync: tear-free, paced by vblank",
                        PresentMode::Immediate => "immediate: async flips, tears, no vblank wait",
                    }
                );
                if requested == PresentMode::Immediate && actual == PresentMode::Vsync {
                    eprintln!(
                        "        asked for immediate, but the driver has no DRM_CAP_ASYNC_PAGE_FLIP"
                    );
                }
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
        input.set_layout_from_system();
        for device in input.devices() {
            eprintln!("input   {}: {}", device.capabilities(), device.name());
        }
        eprintln!(
            "cap     no more often than every {:.1} ms",
            frame.as_secs_f64() * 1000.0
        );
        eprintln!("\npointer to move, click, T for theme, Escape or Q to quit\n");

        let mut poll_fds = wait_fds(&input);

        let mut tracker = DamageTracker::new(size);
        let mut scene = Scene::new(size, input.pointer());

        let deadline = Instant::now() + Duration::from_secs(seconds);
        let mut next_frame = Instant::now();
        let mut events = Vec::new();
        let mut frames = 0u64;
        let mut woke = 0u64;
        let started = Instant::now();

        let mut queued_us: Vec<u32> = Vec::new();
        let mut wait_us: Vec<u32> = Vec::new();
        let mut raster_us: Vec<u32> = Vec::new();
        let mut interval_us: Vec<u32> = Vec::new();
        let mut damage_px: Vec<u32> = Vec::new();
        let mut last_present: Option<Instant> = None;

        while !scene.quit && Instant::now() < deadline {
            // Sleep until input arrives or the next frame is due. With nothing
            // moving there is no next frame, so this blocks outright and the
            // process costs nothing until a finger lands on the panel.
            let timeout = if tracker.is_clean() {
                Duration::from_millis(250)
            } else {
                next_frame.saturating_duration_since(Instant::now())
            };
            let spec = Timespec {
                tv_sec: timeout.as_secs() as _,
                tv_nsec: timeout.subsec_nanos() as _,
            };
            if input.devices_changed() {
                poll_fds = wait_fds(&input);
            }
            let _ = poll(&mut poll_fds, Some(&spec));
            woke += 1;

            events.clear();
            input.poll(&mut events);
            scene.apply(&events, &mut tracker);
            scene.damage(&mut tracker);

            if tracker.is_clean() || Instant::now() < next_frame {
                continue;
            }
            next_frame = Instant::now() + frame;

            // Wait for the display before reading input, not after. On a
            // page-flipping backend this blocks for the rest of the refresh
            // period, and anything read beforehand is that much staler by the time
            // it reaches the screen.
            let wait_start = Instant::now();
            let mut target = surface.acquire()?;
            wait_us.push(wait_start.elapsed().as_micros().min(u32::MAX as u128) as u32);

            // Now, as late as possible, take whatever arrived during the wait.
            events.clear();
            input.poll(&mut events);
            if let Some(age) = input.last_event_age() {
                queued_us.push(age.as_micros().min(u32::MAX as u128) as u32);
            }
            scene.apply(&events, &mut tracker);
            scene.damage(&mut tracker);

            let raster_start = Instant::now();
            let mut regions = [Rect::ZERO; MAX_DAMAGE_RECTS];
            let count = {
                let resolved = tracker.resolve(target.age());
                regions[..resolved.len()].copy_from_slice(resolved);
                resolved.len()
            };
            let damage = &regions[..count];
            damage_px.push(damage.iter().map(Rect::area).sum::<u64>() as u32);

            {
                let mut canvas = Canvas::new(&mut target);
                for region in damage {
                    let mut clipped = canvas.with_clip(*region);
                    scene.paint(&mut clipped);
                }
            }

            drop(target);
            surface.present(damage)?;
            tracker.end_frame();
            scene.presented();
            frames += 1;

            let now = Instant::now();
            raster_us.push(now.duration_since(raster_start).as_micros() as u32);
            if let Some(previous) = last_present.replace(now) {
                interval_us.push(now.duration_since(previous).as_micros() as u32);
            }
        }

        let elapsed = started.elapsed().as_secs_f64();
        eprintln!(
            "\n{frames} frames over {elapsed:.1}s ({:.1}/s), {woke} wake-ups, {} clicks",
            frames as f64 / elapsed,
            scene.clicks
        );
        report("queued     hardware..read  ", &mut queued_us);
        report("wait       for the display ", &mut wait_us);
        report("raster     draw..present   ", &mut raster_us);
        report("interval   present..present", &mut interval_us);
        report_raw("damage     pixels per frame", &mut damage_px);

        if scene.motion_events > 0 {
            eprintln!(
                "\n{} pointer events moved {} px = {:.2} px per event, roughly {:.0}/s",
                scene.motion_events,
                scene.motion_pixels,
                scene.motion_pixels as f64 / scene.motion_events as f64,
                scene.motion_events as f64 / elapsed
            );
        }

        Ok(())
    }

    /// Renders held modifiers, or nothing when none are.
    fn modifier_suffix(modifiers: Modifiers) -> String {
        if modifiers.is_empty() {
            return String::new();
        }
        let mut held = Vec::new();
        for (bit, name) in [
            (Modifiers::CTRL, "ctrl"),
            (Modifiers::ALT, "alt"),
            (Modifiers::SHIFT, "shift"),
            (Modifiers::SUPER, "super"),
        ] {
            if modifiers.contains(bit) {
                held.push(name);
            }
        }
        format!("  [{}]", held.join("+"))
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
    fn report_raw(label: &str, samples: &mut [u32]) {
        if samples.is_empty() {
            return;
        }
        samples.sort_unstable();
        let at = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize] as f64;
        eprintln!(
            "{label}  n={:<5} p50 {:>8.0}      p95 {:>8.0}      max {:>8.0}",
            samples.len(),
            at(0.50),
            at(0.95),
            at(1.0)
        );
    }

    /// The descriptors to wait on, rebuilt when the device set changes.
    ///
    /// Not built once: a device that appears after startup — a wireless mouse woken
    /// minutes into a run — is opened by the next `poll`, and a list made before that
    /// neither names it nor wakes for it.
    fn wait_fds(input: &InputBackend) -> Vec<PollFd<'static>> {
        input
            .raw_fds()
            .into_iter()
            // SAFETY: `input` holds every one of these open, and closes one only in a
            // rescan — which sets the flag that rebuilds this list before the next
            // `poll`.
            .map(|fd| unsafe { BorrowedFd::borrow_raw(fd) })
            .map(|fd| PollFd::from_borrowed_fd(fd, PollFlags::IN))
            .collect()
    }
}
