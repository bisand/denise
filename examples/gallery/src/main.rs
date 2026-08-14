//! Every widget live, and a theme editor beside them.
//!
//! ```text
//! cargo run -p gallery
//! cargo run -p gallery -- --font /usr/share/fonts/truetype/dejavu/DejaVuSans.ttf
//! cargo run -p gallery -- --snapshot shot.ppm
//! cargo run -p gallery -- --snapshot 2x.ppm --size 2560x1600 --scale 2
//! cargo run -p gallery --no-default-features --features kiosk     # the display
//! ```
//!
//! `--size` is the surface, in physical pixels, and `--scale` is how many of
//! those go to a logical one — the pair a HiDPI display hands a window. A
//! *window* is not told either: it asks the backend, and the layout is written
//! in logical pixels regardless.
//!
//! The proof-of-concept panel: everything the toolkit ships, on one screen,
//! with a live theme editor driving [`Ui::set_theme`] the way an application
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

use app::{App, Message};
use denise::Size;

const WINDOW: Size = Size::new(1280, 800);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut font: Option<String> = None;
    let mut snapshot: Option<String> = None;
    // Kiosk builds have no window to close, so a run length is the way out.
    // Zero means "until Escape".
    let mut seconds: u64 = 0;
    let mut size = WINDOW;
    // Snapshots have no display to ask, so the scale is an argument there.
    // A window gets the real one from the backend; see `window_backend`.
    let mut scale: f32 = 1.0;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--font" => font = args.next(),
            "--snapshot" => snapshot = Some(args.next().unwrap_or_else(|| "gallery.ppm".into())),
            "--seconds" => seconds = args.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            "--scale" => scale = args.next().and_then(|s| s.parse().ok()).unwrap_or(1.0),
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

    if let Some(out) = snapshot {
        return write_snapshot(&mut App::new(size, scale, font), size, &out).map_err(Into::into);
    }
    backend::run(font, size, seconds)
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
    use super::{App, Font};
    use denise::{DamageTracker, ElementState, Frame, InputEvent, KeyCode, Rect};
    use denise_winit::{DeniseApp, WindowConfig, run_with};

    pub fn run(
        font: Font,
        size: denise::Size,
        _seconds: u64,
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
                let mut app = App::new(surface, scale, font);
                // The window system already draws a pointer; the tree must not
                // draw a second one over it.
                app.ui.show_cursor(false);
                Gallery(app)
            },
        )?;
        Ok(())
    }

    struct Gallery(App);

    impl DeniseApp for Gallery {
        fn update(&mut self, events: &[InputEvent], damage: &mut DamageTracker) {
            for event in events {
                if let InputEvent::Key {
                    code: KeyCode::F2,
                    state: ElementState::Down,
                    ..
                } = event
                {
                    self.0
                        .on_message(super::Message::UseTheme(self.0.next_builtin()));
                }
            }

            self.0.ui.handle(events);
            let now = self.0.elapsed_ms();
            self.0.ui.tick(now);
            self.0.handle(now);

            if self.0.ui.needs_paint() {
                damage.add_full();
            }
        }

        fn render(&mut self, frame: &mut Frame<'_>, _damage: &[Rect]) {
            self.0.ui.paint(frame);
            self.0.ui.presented();
        }
    }
}

/// The display itself, on a Linux machine with no desktop.
#[cfg(all(feature = "kiosk", target_os = "linux"))]
mod kiosk_backend {
    use std::os::fd::BorrowedFd;
    use std::time::{Duration, Instant};

    use super::{App, Font, Message};
    use bare_linux::{Display, capture, mute_console, open_input, poll_timeout};
    use denise::{ElementState, InputEvent, InputSource, KeyCode, Surface};
    use rustix::event::{PollFd, PollFlags, poll};

    /// Where F12 writes. `/tmp` because a kiosk image is very often read-only
    /// everywhere else.
    const SHOT_PATH: &str = "/tmp/denise-gallery.ppm";

    pub fn run(
        font: Font,
        _size: denise::Size,
        seconds: u64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // A display's size is the one it has; `--size` is a window thing.
        let mut surface = Display::open(bare_linux::PresentMode::Immediate)?;
        let size = surface.size();
        let (mut input, _keymap) = open_input(size)?;
        let _console = mute_console();

        // Built at the display's size, not a window's, and at its scale — which
        // a panel wired straight to a framebuffer reports as 1.
        let scale = surface.scale_factor();
        let mut app = App::new(size, scale, font);
        eprintln!("\nTab moves, F2 theme, F12 screenshot, Escape quits\n");

        let raw_fds = input.raw_fds();
        let borrowed: Vec<BorrowedFd<'_>> = raw_fds
            .iter()
            // SAFETY: every descriptor belongs to `input`, which outlives this
            // loop and holds each device open until the process exits.
            .map(|&fd| unsafe { BorrowedFd::borrow_raw(fd) })
            .collect();
        let mut poll_fds: Vec<PollFd<'_>> = borrowed
            .iter()
            .map(|fd| PollFd::new(fd, PollFlags::IN))
            .collect();

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
            poll(&mut poll_fds, timeout.as_ref())?;

            events.clear();
            input.poll(&mut events);
            if !shortcuts(&mut app, &events, &mut shoot) {
                break;
            }
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
                KeyCode::Escape => return false,
                KeyCode::F2 => app.on_message(Message::UseTheme(app.next_builtin())),
                KeyCode::F12 => *shoot = true,
                _ => {}
            }
        }
        true
    }
}
