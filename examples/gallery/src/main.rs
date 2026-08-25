//! Every widget live, and a theme editor beside them.
//!
//! ```text
//! cargo run -p gallery
//! cargo run -p gallery -- --font /usr/share/fonts/truetype/dejavu/DejaVuSans.ttf
//! cargo run -p gallery -- --motion 33          # 30 fps animation
//! cargo run -p gallery -- --motion off         # reduced motion
//! cargo run -p gallery -- --snapshot shot.ppm
//! cargo run -p gallery -- --snapshot 2x.ppm --size 2560x1600 --scale 2
//! cargo run -p gallery --release -- --scroll-bench --size 1920x1080
//! cargo run -p gallery -- --keyboard          # the on-screen keyboard, up
//! cargo run -p gallery --no-default-features --features kiosk     # the display
//! cargo run -p gallery --no-default-features --features kiosk -- --present vsync
//! ```
//!
//! `--size` is the surface, in physical pixels, and `--scale` is how many of
//! those go to a logical one — the pair a HiDPI display hands a window. A
//! *window* is not told either: it asks the backend, and the layout is written
//! in logical pixels regardless.
//!
//! `--motion` is the animation rate, in milliseconds between samples, or `off`.
//! It costs what a spinner costs: this gallery keeps one turning, and halving
//! the rate halves what an otherwise idle window spends.
//!
//! `--present` chooses how a frame reaches the screen. **Vsync is the default**,
//! measured rather than assumed: on a Pi 3 the vc4's async flips tear visibly
//! enough to read as flicker, and a panel is read rather than aimed at — the
//! 17 ms a paced flip costs is invisible where a tear is not.
//! `--present immediate` asks for the latency back.
//!
//! It stays a flag because it answers a question on hardware that nothing else
//! can: the two faults it separates look alike to a person watching the panel.
//! A **tear** is one frame torn across a seam; a **stale buffer** is two frames
//! alternating. Both read as flicker. If `vsync` removes it, it was the flip;
//! if it does not, the flip was never involved and the damage is wrong
//! somewhere. That is how the keyboard's flicker was diagnosed, and the answer
//! was both — a repaint bug did most of it and the flip did the rest. Kiosk
//! only: a window's presentation belongs to whatever is compositing it.
//!
//! The proof-of-concept panel: everything the toolkit ships, on one screen,
//! with a live theme editor driving [`Ui::set_theme`](denise_ui::Ui::set_theme) the way an application
//! would. The editing is the demonstration — nine seed colours go in, a
//! derived, contrast-checked theme comes out, and every widget follows
//! because widgets name roles, never colours.
//!
//! The application lives in `app.rs` and never learns what it is running on.
//! This file holds the two backends, the same pair as `table-editor` and
//! chosen the same way: a cargo feature, at compile time, because no runtime
//! probe can tell a kiosk Pi from a desktop Pi. See that example's header for
//! the argument in full.

mod app;
mod clock;

use app::{App, Message};
use denise::Size;
use denise_ui::Motion;

const WINDOW: Size = Size::new(1280, 800);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut font: Option<String> = None;
    let mut snapshot: Option<String> = None;
    let mut scroll_bench: Option<usize> = None;
    let mut keyboard = false;
    // Kiosk builds have no window to close, so a run length is the way out.
    // Zero means "until Escape".
    let mut seconds: u64 = 0;
    let mut size = WINDOW;
    // Snapshots have no display to ask, so the scale is an argument there.
    // A window gets the real one from the backend; see `window_backend`.
    let mut scale: f32 = 1.0;
    let mut motion = Motion::default();
    // Vsync is the default, measured on a Pi 3 rather than assumed: the vc4's
    // async flips tear visibly enough to read as flicker. See the header for
    // what asking for the other one is worth.
    let mut vsync = true;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--font" => font = args.next(),
            "--present" => {
                vsync = match args.next().as_deref() {
                    Some("vsync") => true,
                    Some("immediate") => false,
                    other => {
                        eprintln!("--present takes vsync or immediate, not {other:?}");
                        return Ok(());
                    }
                }
            }
            "--snapshot" => snapshot = Some(args.next().unwrap_or_else(|| "gallery.ppm".into())),
            // The measurement behind #46, so that anybody can take it again.
            "--scroll-bench" => {
                scroll_bench = Some(args.next().and_then(|s| s.parse().ok()).unwrap_or(60));
            }
            "--seconds" => seconds = args.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            "--keyboard" => keyboard = true,
            "--scale" => scale = args.next().and_then(|s| s.parse().ok()).unwrap_or(1.0),
            "--motion" => {
                motion = match args.next().as_deref() {
                    Some("off" | "none") => Motion::None,
                    Some(ms) => Motion::Every(ms.parse().unwrap_or(16)),
                    None => Motion::default(),
                }
            }
            // A window other than the default, mostly for reviewing snapshots
            // of the parts a default-sized first frame leaves below the fold.
            "--size" => {
                if let Some((w, h)) = args.next().and_then(|s| {
                    let (w, h) = s.split_once('x')?;
                    Some((w.parse().ok()?, h.parse().ok()?))
                }) {
                    size = Size::new(w, h);
                }
            }
            other => {
                eprintln!("unknown argument {other}");
                return Ok(());
            }
        }
    }

    let font = system_font::load(font.as_deref());

    if let Some(rounds) = scroll_bench {
        scroll_bench_run(size, scale, font, motion, rounds);
        return Ok(());
    }

    if let Some(out) = snapshot {
        let mut app = App::new(size, scale, font, motion);
        if keyboard {
            // The button moves the caret; the keyboard follows on the next
            // frame, and a shelf slides. A snapshot has no frames of its own, so
            // it runs a few.
            app.on_message(Message::ToggleKeyboard);
            for now in [1, 200, 400, 10_000] {
                app.ui.tick(now);
                app.handle(now);
            }
        }
        return write_snapshot(&mut app, size, &out).map_err(Into::into);
    }
    backend::run(font, size, seconds, motion, vsync, keyboard)
}

/// Times `Ui::paint` alone, on a scrolled frame, with no display attached.
///
/// The measurement [#46](https://github.com/bisand/denise/issues/46) is about.
/// Nothing here waits for a vblank, reads an input device or copies a buffer to
/// a screen: the number is the cost of *rasterising* one scrolled frame of a
/// real tree, which is the thing that does not fit in a Pi's frame budget at
/// 1080p.
///
/// ```text
/// gallery --scroll-bench            # 60 scrolled frames at the default size
/// gallery --scroll-bench 200 --size 1920x1080
/// ```
fn scroll_bench_run(size: Size, scale: f32, font: Font, motion: Motion, rounds: usize) {
    let mut app = App::new(size, scale, font, motion);
    // The spinner and the clock would both ask for frames of their own, and
    // this is a measurement of scrolling.
    app.on_message(Message::Spin(false));

    let mut pixels = vec![0u32; (size.width * size.height) as usize];
    // Scoped, because the closure borrows the buffer and the measurement after
    // the loop needs it back.
    let (viewport, rects, took) = {
        let mut paint = |app: &mut App, age| {
            let mut frame = denise::Frame::new(
                &mut pixels,
                size,
                size.width,
                denise::PixelFormat::Xrgb8888,
                age,
            )
            .expect("frame");
            let started = std::time::Instant::now();
            app.ui.paint(&mut frame);
            let took = started.elapsed();
            drop(frame);
            app.ui.presented();
            took
        };

        // Settle first, so what is timed below is the scrolled frame alone.
        for _ in 0..8 {
            app.ui.tick(0);
            paint(&mut app, denise::BufferAge::Undefined);
        }

        let content = app.content_viewport();
        let viewport = app.ui.bounds(content).unwrap_or(denise::Rect::ZERO);

        let mut took: Vec<u128> = Vec::with_capacity(rounds);
        let mut rects = 0usize;
        for round in 0..rounds {
            // Down, then back up when it runs out, so a long run keeps scrolling
            // rather than sitting at the bottom repainting nothing.
            let dy = if (round / 40) % 2 == 0 { 20 } else { -20 };
            app.ui.scroll_by(content, 0, dy);
            rects = rects.max(app.ui.pending_damage().len());
            app.ui.tick(1_000 + round as u64 * 16);
            took.push(paint(&mut app, denise::BufferAge::Frames(2)).as_micros());
        }
        took.sort_unstable();
        (viewport, rects, took)
    };

    let surface = i64::from(size.width) * i64::from(size.height);
    let covered = i64::from(viewport.width) * i64::from(viewport.height);

    // What the fast path in #46 would have to pay instead of rasterising: the
    // still-valid rows moved within the buffer. Measured here rather than
    // assumed, because it is the number that decides whether scroll-by-blit can
    // reach the frame budget at all — and on a write-combined framebuffer a
    // read is not the same price as a write.
    let stride = size.width as usize;
    let rows = viewport.height.max(0) as usize;
    let width = viewport.width.max(0) as usize;
    let shift = 20usize;
    let mut copied: Vec<u128> = Vec::with_capacity(16);
    for _ in 0..16 {
        let started = std::time::Instant::now();
        for row in shift..rows {
            let from = (viewport.y as usize + row) * stride + viewport.x as usize;
            let to = (viewport.y as usize + row - shift) * stride + viewport.x as usize;
            pixels.copy_within(from..from + width, to);
        }
        copied.push(started.elapsed().as_micros());
    }
    copied.sort_unstable();

    println!("gallery --scroll-bench, {rounds} scrolled frames");
    println!("  surface   {}x{}", size.width, size.height);
    println!(
        "  viewport  {}x{} at {},{} — {}% of the surface",
        viewport.width,
        viewport.height,
        viewport.x,
        viewport.y,
        covered * 100 / surface.max(1)
    );
    println!("  damage    {rects} rect(s) per scroll");
    println!(
        "  paint     median {:.2} ms   min {:.2}   max {:.2}",
        took[took.len() / 2] as f64 / 1000.0,
        took[0] as f64 / 1000.0,
        took[took.len() - 1] as f64 / 1000.0,
    );
    println!(
        "  memmove   median {:.2} ms  — {} rows of {} words, what a blit would cost instead",
        copied[copied.len() / 2] as f64 / 1000.0,
        rows.saturating_sub(shift),
        width,
    );
}

/// Draws one frame into a PPM, with no window and no event loop.
fn write_snapshot(app: &mut App, size: Size, path: &str) -> std::io::Result<()> {
    use std::io::Write as _;

    let mut pixels = vec![0u32; (size.width * size.height) as usize];
    {
        let mut frame = denise::Frame::new(
            &mut pixels,
            size,
            size.width,
            denise::PixelFormat::Xrgb8888,
            denise::BufferAge::Undefined,
        )
        .expect("frame");
        app.ui.paint(&mut frame);
    }

    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(out, "P6\n{} {}\n255\n", size.width, size.height)?;
    for word in &pixels {
        out.write_all(&[(word >> 16) as u8, (word >> 8) as u8, *word as u8])?;
    }
    out.flush()?;
    eprintln!("wrote {path} at {}x{}", size.width, size.height);
    Ok(())
}

/// What either backend hands the app builder.
type Font = Option<(String, Box<dyn denise_text::GlyphSource>)>;

// Exactly one backend; the rule is table-editor's, written out there.
#[cfg(all(feature = "kiosk", target_os = "linux"))]
use kiosk_backend as backend;

#[cfg(all(feature = "desktop", not(all(feature = "kiosk", target_os = "linux"))))]
use window_backend as backend;

#[cfg(not(any(all(feature = "kiosk", target_os = "linux"), feature = "desktop")))]
compile_error!(
    "gallery has no backend to draw with. Enable `desktop` for a window, or \
     `kiosk` for the display itself, which needs Linux."
);

/// A window, on any desktop.
#[cfg(all(feature = "desktop", not(all(feature = "kiosk", target_os = "linux"))))]
mod window_backend {
    use std::time::Duration;

    use super::{App, Font};
    use denise::{DamageTracker, ElementState, Frame, InputEvent, KeyCode, Rect};
    use denise_winit::{DeniseApp, WindowConfig, run_with};

    use super::Message;

    pub fn run(
        font: Font,
        size: denise::Size,
        _seconds: u64,
        motion: denise_ui::Motion,
        // A window does not choose: the compositor owns the flip, and softbuffer
        // has nowhere to put the preference even if it did.
        _vsync: bool,
        keyboard: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // `run_with`, not `run`: the tree is built after the window exists, which
        // is the first moment the display's scale factor is knowable. On a Retina
        // Mac that is 2, the surface behind a 1280×800 window is 2560×1600, and
        // building for it is the difference between a gallery you can read and
        // one drawn at half size.
        run_with(
            WindowConfig {
                title: "Denise — gallery".into(),
                size,
                ..WindowConfig::default()
            },
            move |surface, scale| {
                let mut app = App::new(surface, scale, font, motion);
                if keyboard {
                    app.on_message(Message::ToggleKeyboard);
                }
                // The window system already draws a pointer; the tree must not
                // draw a second one over it.
                app.ui.show_cursor(false);
                Gallery { app, exit: false }
            },
        )?;
        Ok(())
    }

    struct Gallery {
        app: App,
        exit: bool,
    }

    impl Gallery {
        /// Whether Escape is this application's to act on, or the tree's.
        ///
        /// The tree claims Escape for exactly two things — an open popup and a
        /// drawer on top — and the gallery's drawer says so on its own face
        /// ("Escape, or a press on the dim, slides it out"). A dialog is a pushed
        /// scene with its own buttons, and quitting out from under one would be
        /// answering a question nobody answered. So the key quits only when there
        /// is nothing on screen to dismiss.
        fn escape_is_mine(&self) -> bool {
            !self.app.ui.popup_open()
                && !self.app.ui.drawer_open()
                && self.app.ui.scene_count() == 1
        }
    }

    impl DeniseApp for Gallery {
        fn update(&mut self, events: &[InputEvent], damage: &mut DamageTracker) {
            for event in events {
                let InputEvent::Key {
                    code,
                    state: ElementState::Down,
                    ..
                } = event
                else {
                    continue;
                };
                match code {
                    KeyCode::F2 => self
                        .app
                        .on_message(super::Message::UseTheme(self.app.next_builtin())),
                    KeyCode::Escape if self.escape_is_mine() => self.exit = true,
                    _ => {}
                }
            }

            self.app.keyboard_input(events);
            self.app.ui.handle(events);
            let now = self.app.elapsed_ms();
            self.app.ui.tick(now);
            self.app.handle(now);

            // The tree's own rectangles, not `add_full`. The window copies
            // exactly what changed, which is the thing this toolkit is about —
            // and at 2x it is the difference between copying a caret and
            // copying sixteen megabytes a second. An empty list with
            // `needs_paint` set means the tree wants everything.
            if self.app.ui.needs_paint() {
                let pending = self.app.ui.pending_damage();
                if pending.is_empty() {
                    damage.add_full();
                } else {
                    for rect in pending {
                        damage.add(*rect);
                    }
                }
            }
        }

        fn render(&mut self, frame: &mut Frame<'_>, _damage: &[Rect]) {
            self.app.ui.paint(frame);
            self.app.ui.presented();
        }

        fn exit_requested(&self) -> bool {
            self.exit
        }

        /// What the tree asked for, which on this screen is the spinner: 50 ms,
        /// not the 16 the loop would otherwise take. Woken three times as often
        /// as it moves, a spinner reports a repaint every time and every one of
        /// those is a full-surface upload on macOS.
        fn next_frame_in(&self) -> Option<Duration> {
            let now = self.app.elapsed_ms();
            self.app
                .ui
                .next_wake_ms()
                .map(|wake| Duration::from_millis(wake.saturating_sub(now)))
        }
    }
}

/// The display itself, on a Linux machine with no desktop.
#[cfg(all(feature = "kiosk", target_os = "linux"))]
mod kiosk_backend {
    use std::time::{Duration, Instant};

    use super::{App, Font, Message};
    use bare_linux::{Display, Waits, capture, mute_console, open_input, poll_timeout};
    use denise::{ElementState, InputEvent, InputSource, KeyCode, Surface};

    /// Where F12 writes. `/tmp` because a kiosk image is very often read-only
    /// everywhere else.
    const SHOT_PATH: &str = "/tmp/denise-gallery.ppm";

    pub fn run(
        font: Font,
        _size: denise::Size,
        seconds: u64,
        motion: denise_ui::Motion,
        vsync: bool,
        keyboard: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // A display's size is the one it has; `--size` is a window thing.
        let mode = if vsync {
            bare_linux::PresentMode::Vsync
        } else {
            bare_linux::PresentMode::Immediate
        };
        let mut surface = Display::open(mode)?;
        let size = surface.size();
        let (mut input, _keymap) = open_input(size)?;
        let _console = mute_console();

        // Built at the display's size, not a window's, and at its scale — which
        // a panel wired straight to a framebuffer reports as 1.
        let scale = surface.scale_factor();
        let mut app = App::new(size, scale, font, motion);
        if keyboard {
            app.on_message(Message::ToggleKeyboard);
        }
        eprintln!("\nTab moves, F2 theme, F12 screenshot, Escape quits\n");

        // Refreshed by `wait` whenever a device is opened or closed, which is
        // why this is a `Waits` and not a list built once.
        let mut waits = Waits::new(&input);

        // The first frame, before anything is allowed to block; see
        // table-editor for the bug this prevents.
        {
            let mut frame = surface.acquire()?;
            app.ui.paint(&mut frame);
            drop(frame);
            surface.present(app.ui.damage())?;
            app.ui.presented();
        }

        let started = Instant::now();
        let deadline = started
            + if seconds == 0 {
                Duration::from_secs(60 * 60 * 24)
            } else {
                Duration::from_secs(seconds)
            };
        let mut events = Vec::new();
        let mut shoot = false;

        while Instant::now() < deadline {
            // The app's clock, not a second one here: the tree's ticks come
            // from one place on every backend.
            let timeout = poll_timeout(app.ui.next_wake_ms(), app.elapsed_ms(), deadline);
            waits.wait(&mut input, timeout.as_ref())?;

            events.clear();
            input.poll(&mut events);
            if !shortcuts(&mut app, &events, &mut shoot) {
                break;
            }
            app.keyboard_input(&events);
            app.ui.handle(&events);
            app.ui.tick(app.elapsed_ms());
            let now = app.elapsed_ms();
            app.handle(now);

            if !app.ui.needs_paint() && !shoot {
                continue;
            }

            let mut frame = surface.acquire()?;
            app.ui.paint(&mut frame);
            if core::mem::take(&mut shoot) {
                match capture(&frame, SHOT_PATH) {
                    Ok(()) => eprintln!("wrote {SHOT_PATH}"),
                    Err(e) => eprintln!("could not write {SHOT_PATH}: {e}"),
                }
            }
            drop(frame);
            surface.present(app.ui.damage())?;
            app.ui.presented();
        }
        Ok(())
    }

    /// Keys this application claims before the tree sees them.
    fn shortcuts(app: &mut App, events: &[InputEvent], shoot: &mut bool) -> bool {
        for event in events {
            let InputEvent::Key {
                code,
                state: ElementState::Down,
                ..
            } = event
            else {
                continue;
            };
            match code {
                // Escape quits, but the tree gets it first when there is
                // something to dismiss — the drawer says "Escape, or a press on
                // the dim, slides it out" on its own face, and a dropdown closes
                // on the same key. The window backend asks exactly this, which is
                // the point: one application, one answer, two displays.
                KeyCode::Escape
                    if !app.ui.popup_open()
                        && !app.ui.drawer_open()
                        && app.ui.scene_count() == 1 =>
                {
                    return false;
                }
                KeyCode::F2 => app.on_message(Message::UseTheme(app.next_builtin())),
                KeyCode::F12 => *shoot = true,
                _ => {}
            }
        }
        true
    }
}
