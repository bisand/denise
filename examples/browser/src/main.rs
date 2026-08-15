//! A small web browser, rendered entirely with Denise widgets.
//!
//! ```text
//! cargo run -p browser                                  # the welcome page
//! cargo run -p browser -- https://example.com
//! cargo run -p browser -- examples/browser/fixtures/basic.html
//! cargo run -p browser -- --snapshot shot.ppm https://example.com
//! cargo run -p browser --no-default-features --features kiosk   # the display
//! ```
//!
//! No JavaScript, no floats, no promises of fidelity — a page comes out
//! readable, its links click, and every visible thing is a Denise widget:
//! the URL bar is `TextInput`, the page text is a custom widget over the
//! shared text engine, and the kiosk build is the same application on a
//! bare panel.
//!
//! The application lives in `app.rs` and never learns what it is running
//! on. This file holds the two backends, the same pair as the gallery and
//! chosen the same way: a cargo feature, at compile time, because no
//! runtime probe can tell a kiosk Pi from a desktop Pi.

mod app;
mod css;
mod dom;
mod forms;
mod history;
mod layout;
mod net;
mod page;
mod style;
mod textflow;

use app::App;
use denise::Size;
use denise_ui::Motion;

const WINDOW: Size = Size::new(1100, 800);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut font: Option<String> = None;
    let mut snapshot: Option<String> = None;
    let mut seconds: u64 = 0;
    let mut size = WINDOW;
    let mut scale: f32 = 1.0;
    let mut motion = Motion::default();
    let mut start: Option<String> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--font" => font = args.next(),
            "--snapshot" => snapshot = Some(args.next().unwrap_or_else(|| "browser.ppm".into())),
            "--seconds" => seconds = args.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            "--scale" => scale = args.next().and_then(|s| s.parse().ok()).unwrap_or(1.0),
            "--motion" => {
                motion = match args.next().as_deref() {
                    Some("off" | "none") => Motion::None,
                    Some(ms) => Motion::Every(ms.parse().unwrap_or(16)),
                    None => Motion::default(),
                }
            }
            "--size" => {
                if let Some((w, h)) = args.next().and_then(|s| {
                    let (w, h) = s.split_once('x')?;
                    Some((w.parse().ok()?, h.parse().ok()?))
                }) {
                    size = Size::new(w, h);
                }
            }
            other if !other.starts_with('-') && start.is_none() => start = Some(other.to_string()),
            other => {
                eprintln!("unknown argument {other}");
                return Ok(());
            }
        }
    }

    let font = system_font::load(font.as_deref());

    if let Some(out) = snapshot {
        let mut app = App::new(size, scale, font, motion, start);
        return write_snapshot(&mut app, size, &out).map_err(Into::into);
    }
    backend::run(font, size, seconds, motion, start)
}

/// One frame into a PPM — after the page has actually arrived, which for a
/// network address means pumping the loop the backends would otherwise run.
fn write_snapshot(app: &mut App, size: Size, path: &str) -> std::io::Result<()> {
    use std::io::Write as _;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let now = app.elapsed_ms();
        app.ui.tick(now);
        app.handle(now);
        if !app.loading() {
            break;
        }
        if std::time::Instant::now() > deadline {
            eprintln!("the page did not arrive in time; capturing what there is");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }

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
    "browser has no backend to draw with. Enable `desktop` for a window, or \
     `kiosk` for the display itself, which needs Linux."
);

/// While a fetch is in flight the loop must look at the channel now and
/// then; this is the cadence. Idle, nothing polls — the toolkit's rule.
#[allow(dead_code)]
const LOADING_POLL_MS: u64 = 40;

/// A window, on any desktop.
#[cfg(all(feature = "desktop", not(all(feature = "kiosk", target_os = "linux"))))]
mod window_backend {
    use std::time::Duration;

    use super::{App, Font, LOADING_POLL_MS};
    use denise::{DamageTracker, ElementState, Frame, InputEvent, KeyCode, Rect};
    use denise_winit::{DeniseApp, WindowConfig, run_with};

    pub fn run(
        font: Font,
        size: denise::Size,
        _seconds: u64,
        motion: denise_ui::Motion,
        start: Option<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        run_with(
            WindowConfig {
                title: "Denise — browser".into(),
                size,
                ..WindowConfig::default()
            },
            move |surface, scale| {
                let mut app = App::new(surface, scale, font, motion, start);
                app.ui.show_cursor(false);
                Browser { app, exit: false }
            },
        )?;
        Ok(())
    }

    struct Browser {
        app: App,
        exit: bool,
    }

    impl Browser {
        /// Escape quits only when there is nothing on screen to dismiss;
        /// the tree gets it first for popups and drawers, as everywhere.
        fn escape_is_mine(&self) -> bool {
            !self.app.ui.popup_open()
                && !self.app.ui.drawer_open()
                && self.app.ui.scene_count() == 1
        }
    }

    impl DeniseApp for Browser {
        fn update(&mut self, events: &[InputEvent], damage: &mut DamageTracker) {
            for event in events {
                if let InputEvent::Key {
                    code: KeyCode::Escape,
                    state: ElementState::Down,
                    ..
                } = event
                    && self.escape_is_mine()
                {
                    self.exit = true;
                }
            }
            self.app.claim(events);
            self.app.ui.handle(events);
            let now = self.app.elapsed_ms();
            self.app.ui.tick(now);
            self.app.handle(now);

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

        fn next_frame_in(&self) -> Option<Duration> {
            let now = self.app.elapsed_ms();
            let wake = self
                .app
                .ui
                .next_wake_ms()
                .map(|wake| Duration::from_millis(wake.saturating_sub(now)));
            if self.app.loading() {
                let poll = Duration::from_millis(LOADING_POLL_MS);
                Some(wake.map_or(poll, |w| w.min(poll)))
            } else {
                wake
            }
        }
    }
}

/// The display itself, on a Linux machine with no desktop.
#[cfg(all(feature = "kiosk", target_os = "linux"))]
mod kiosk_backend {
    use std::os::fd::BorrowedFd;
    use std::time::{Duration, Instant};

    use super::{App, Font, LOADING_POLL_MS};
    use bare_linux::{Display, capture, mute_console, open_input, poll_timeout};
    use denise::{ElementState, InputEvent, InputSource, KeyCode, Surface};
    use rustix::event::{PollFd, PollFlags, poll};

    /// Where F12 writes. `/tmp` because a kiosk image is very often
    /// read-only everywhere else.
    const SHOT_PATH: &str = "/tmp/denise-browser.ppm";

    pub fn run(
        font: Font,
        _size: denise::Size,
        seconds: u64,
        motion: denise_ui::Motion,
        start: Option<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut surface = Display::open(bare_linux::PresentMode::Immediate)?;
        let size = surface.size();
        let (mut input, _keymap) = open_input(size)?;
        let _console = mute_console();

        let scale = surface.scale_factor();
        let mut app = App::new(size, scale, font, motion, start);
        eprintln!("\nTab moves, Alt+arrows go back and forward, F12 screenshot, Escape quits\n");

        let raw_fds = input.raw_fds();
        let borrowed: Vec<BorrowedFd<'_>> = raw_fds
            .iter()
            // SAFETY: every descriptor belongs to `input`, which outlives
            // this loop and holds each device open until the process exits.
            .map(|&fd| unsafe { BorrowedFd::borrow_raw(fd) })
            .collect();
        let mut poll_fds: Vec<PollFd<'_>> = borrowed
            .iter()
            .map(|fd| PollFd::new(fd, PollFlags::IN))
            .collect();

        // The first frame, before anything is allowed to block.
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
            let now = app.elapsed_ms();
            // The channel from the fetch thread cannot wake a poll on input
            // descriptors, so while a page is in flight the wait is capped.
            let mut wake = app.ui.next_wake_ms();
            if app.loading() {
                let cap = now + LOADING_POLL_MS;
                wake = Some(wake.map_or(cap, |w| w.min(cap)));
            }
            let timeout = poll_timeout(wake, now, deadline);
            poll(&mut poll_fds, timeout.as_ref())?;

            events.clear();
            input.poll(&mut events);
            if !shortcuts(&mut app, &events, &mut shoot) {
                break;
            }
            app.claim(&events);
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
                KeyCode::Escape
                    if !app.ui.popup_open()
                        && !app.ui.drawer_open()
                        && app.ui.scene_count() == 1 =>
                {
                    return false;
                }
                KeyCode::F12 => *shoot = true,
                _ => {}
            }
        }
        true
    }
}
