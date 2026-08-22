//! A record editor: a scrolling grid, an edit form, validation and a modal.
//!
//! ```text
//! cargo run -p table-editor
//! cargo run -p table-editor -- --font /usr/share/fonts/truetype/dejavu/DejaVuSans.ttf
//! cargo run -p table-editor -- --snapshot shot.ppm
//! cargo run -p table-editor -- --keyboard        # the on-screen keyboard, up
//! ```
//!
//! The same application as `hello`, grown up: about as much user interface as a
//! small internal tool has, and the thing to read once the twenty-line version
//! makes sense.
//!
//! # What it is showing
//!
//! - **A table built from four widgets.** There is no grid widget. A row is a
//!   full-width button with labels on top of it; see `app.rs`.
//! - **A fixed set of row nodes.** Nine of them, however many records there are.
//!   Scrolling changes what they display, not how many exist.
//! - **Real fonts.** `--font` loads a TrueType or OpenType file. Without one it
//!   falls back to the built-in 8×8 bitmap, which is what a panel with 145 KB to
//!   spare and no font file uses.
//! - **A modal that needs no flags.** Deleting opens a dimmed scene; everything
//!   underneath stops taking input because something is above it.
//! - **Rules separate from drawing.** `table.rs` knows nothing about widgets and
//!   is entirely unit tested, which is where the rules of a record editor belong.
//!
//! # One application, several machines
//!
//! `app.rs` never learns where it is running. This file holds the two backends and
//! each is about fifty lines: a window on any desktop, and the display itself on a
//! Linux machine that has no desktop at all.
//!
//! **Which one is decided here, and decided at compile time**, by a cargo feature:
//!
//! ```text
//! cargo run -p table-editor                                            # a window
//! cargo run -p table-editor --no-default-features --features kiosk     # the display
//! ```
//!
//! The toolkit does not choose and offers no way to. It cannot:
//! `aarch64-unknown-linux-gnu` is the same target on a kiosk Pi and on a Pi running
//! the desktop image, so any probe in a library would be wrong half the time — and
//! wrong means a program that opens nothing, on a machine somebody has already
//! shipped. The application knows what it is being built for; nothing below it
//! does.

mod app;
mod table;

use app::{App, Message};
use denise::Size;
use denise_ui::TextStyle;

const WINDOW: Size = Size::new(1000, 470);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut path = "people.csv".to_string();
    let mut font: Option<String> = None;
    let mut snapshot: Option<String> = None;
    // Kiosk builds have no window to close, so a run length is the way out. Zero
    // means "until Escape".
    let mut seconds: u64 = 0;
    let mut keyboard = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--font" => font = args.next(),
            "--snapshot" => snapshot = Some(args.next().unwrap_or_else(|| "table.ppm".into())),
            "--seconds" => seconds = args.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            "--keyboard" => keyboard = true,
            other => path = other.to_string(),
        }
    }

    let table = match std::fs::read_to_string(&path) {
        Ok(text) => table::Table::parse(&text),
        Err(_) => {
            eprintln!("{path} does not exist yet; starting from the sample. Save writes it.");
            table::Table::parse(table::SAMPLE)
        }
    };

    let setup = Setup {
        path,
        table,
        font: system_font::load(font.as_deref()),
    };

    if let Some(out) = snapshot {
        let mut app = setup.build(WINDOW, 1.0);
        if keyboard {
            app.focus_last_field();
            app.handle(0);
            // A shelf slides, and a snapshot has no frames to slide over.
            app.ui.tick(10_000);
            app.handle(10_000);
        }
        return write_snapshot(&mut app, &out).map_err(Into::into);
    }
    backend::run(setup, seconds, keyboard)
}

/// Everything needed to build the tree, held until a backend says how big.
///
/// A window has a size the application chooses; a display has one it is given. So
/// the tree cannot be built here — it is built by whichever backend learns the
/// answer first, and this is what that takes.
pub struct Setup {
    path: String,
    table: table::Table,
    font: Option<(String, Box<dyn denise_text::GlyphSource>)>,
}

impl Setup {
    /// Builds the tree for a surface of `size` physical pixels at `scale`.
    ///
    /// A display reports scale 1 and a window is told what its display says, so
    /// this is the same call on a Pi and on a Retina Mac — with different numbers.
    pub fn build(self, size: Size, scale: f32) -> App {
        let px = |v: u16| ((v as f32) * scale + 0.5) as u16;
        let mut app = App::new(
            size,
            scale,
            self.path,
            self.table,
            TextStyle::built_in(px(16)),
            TextStyle::built_in(px(24)),
        );
        // The font is registered *after* the tree exists, because registering one
        // damages the whole surface — every string on screen may change width —
        // and startup is the cheapest possible time to pay that.
        match self.font {
            Some((name, source)) => {
                eprintln!("using {name}");
                let id = app.ui.add_font(source);
                app.set_font(id);
            }
            None => eprintln!("no TrueType font found; using the built-in 8x8 bitmap font"),
        }
        app
    }
}

/// Draws one frame into a PPM, with no window and no event loop.
fn write_snapshot(app: &mut App, path: &str) -> std::io::Result<()> {
    use std::io::Write as _;

    let mut pixels = vec![0u32; (WINDOW.width * WINDOW.height) as usize];
    {
        let mut frame = denise::Frame::new(
            &mut pixels,
            WINDOW,
            WINDOW.width,
            denise::PixelFormat::Xrgb8888,
            denise::BufferAge::Undefined,
        )
        .expect("frame");
        app.ui.paint(&mut frame);
    }

    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(out, "P6\n{} {}\n255\n", WINDOW.width, WINDOW.height)?;
    for word in &pixels {
        out.write_all(&[(word >> 16) as u8, (word >> 8) as u8, *word as u8])?;
    }
    out.flush()?;
    eprintln!("wrote {path} at {}x{}", WINDOW.width, WINDOW.height);
    Ok(())
}

// ---------------------------------------------------------------- the backends

// Exactly one backend, and the rule for picking it is written out rather than
// left to whichever `use` the compiler reaches first.
//
// `--all-features` turns both on, and CI does that on purpose — a `#[cfg]` that
// compiles in one combination and not another is the rot this whole project keeps
// finding. So "both" has to mean something rather than fail to build, and it means
// kiosk: nobody enables a bare-display backend by accident, and a desktop build is
// what you get by doing nothing.
//
// The kiosk arm carries `target_os = "linux"` because the feature can be switched
// on anywhere and the backend exists nowhere else. The desktop arm negates the
// whole condition, not just the feature, so asking for kiosk on macOS still leaves
// you with a window rather than with nothing at all.
#[cfg(all(feature = "kiosk", target_os = "linux"))]
use kiosk_backend as backend;

#[cfg(all(feature = "desktop", not(all(feature = "kiosk", target_os = "linux"))))]
use window_backend as backend;

#[cfg(not(any(all(feature = "kiosk", target_os = "linux"), feature = "desktop")))]
compile_error!(
    "table-editor has no backend to draw with. Enable `desktop` for a window, or \
     `kiosk` for the display itself, which needs Linux."
);

/// A window, on any desktop. Fifty lines, all of them plumbing.
#[cfg(all(feature = "desktop", not(all(feature = "kiosk", target_os = "linux"))))]
mod window_backend {
    use std::time::Duration;

    use super::{App, Message, Setup, WINDOW};
    use denise::{DamageTracker, ElementState, Frame, InputEvent, KeyCode, Rect};
    use denise_winit::{DeniseApp, WindowConfig, run_with};

    pub fn run(
        setup: Setup,
        _seconds: u64,
        keyboard: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // A window's size is the application's choice, so this one is `WINDOW` —
        // logical, so it covers the same desk on every display. The tree is built
        // from the callback, because the surface behind that window and the scale
        // factor that produced it are not knowable any earlier.
        run_with(
            WindowConfig {
                title: "Denise — table editor".into(),
                size: WINDOW,
                ..WindowConfig::default()
            },
            move |surface, scale| {
                let mut app = setup.build(surface, scale);
                if keyboard {
                    app.focus_first_field();
                }
                // The window system already draws a pointer; the tree must not
                // draw a second one over it. On the kiosk backend there is nothing
                // else drawing one, so it keeps its own — the same tree, a
                // different right answer.
                app.ui.show_cursor(false);
                Editor(app)
            },
        )?;
        Ok(())
    }

    struct Editor(App);

    impl DeniseApp for Editor {
        fn update(&mut self, events: &[InputEvent], damage: &mut DamageTracker) {
            // Keys the application claims before the tree sees them. Everything
            // else — Tab, typing, the caret — is the toolkit's business.
            for event in events {
                if let InputEvent::Key {
                    code,
                    state: ElementState::Down,
                    ..
                } = event
                {
                    match code {
                        KeyCode::ArrowUp if !self.0.is_confirming() => self.0.move_selection(-1),
                        KeyCode::ArrowDown if !self.0.is_confirming() => self.0.move_selection(1),
                        KeyCode::F2 => self.0.on_message(Message::NextTheme),
                        // Escape quits, as it does on the kiosk — but not out from
                        // under the delete confirmation, which is a question that
                        // deserves an answer rather than an exit.
                        // The keyboard is dismissed before quitting is even
                        // considered: a shelf pushes no scene, so nothing in the
                        // tree claims Escape on its behalf.
                        KeyCode::Escape if self.0.keyboard_open() => self.0.dismiss_keyboard(),
                        KeyCode::Escape if !self.0.is_confirming() => self.0.exit = true,
                        _ => {}
                    }
                }
            }

            self.0.ui.handle(events);
            let now = self.0.elapsed_ms();
            self.0.ui.tick(now);
            self.0.handle(now);

            // The tree's rectangles, the same ones the kiosk arm presents.
            // An empty list with `needs_paint` set means everything.
            if self.0.ui.needs_paint() {
                let pending = self.0.ui.pending_damage();
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
            self.0.ui.paint(frame);
            self.0.ui.presented();
        }

        fn exit_requested(&self) -> bool {
            self.0.exit
        }

        /// The same question the kiosk arm answers with `poll_timeout`: how long
        /// may this sleep? A tree with nothing animating says `None`, and the
        /// window then waits for input rather than for a clock.
        fn next_frame_in(&self) -> Option<Duration> {
            let now = self.0.elapsed_ms();
            self.0
                .ui
                .next_wake_ms()
                .map(|wake| Duration::from_millis(wake.saturating_sub(now)))
        }
    }
}

/// The display itself, on a Linux machine with no desktop.
///
/// Same application above this line; a different fifty lines below it. `bare-linux`
/// supplies the pieces — the display with its fbdev fallback, input, the console
/// guard, the poll timeout — and the loop stays here, because where input is read
/// relative to the display wait is a decision with a measurable cost and it is
/// this program's to make.
#[cfg(all(feature = "kiosk", target_os = "linux"))]
mod kiosk_backend {
    use std::time::{Duration, Instant};

    use super::{App, Message, Setup};
    use bare_linux::{Display, Waits, capture, mute_console, open_input, poll_timeout};
    use denise::{ElementState, InputEvent, InputSource, KeyCode, Surface};

    /// Where F12 writes. `/tmp` because a kiosk image is very often read-only
    /// everywhere else.
    const SHOT_PATH: &str = "/tmp/denise-table-editor.ppm";

    pub fn run(
        setup: Setup,
        seconds: u64,
        keyboard: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Immediate rather than vsync: this panel is driven by a keypad and a
        // pointer, so latency matters more to it than tearing does.
        let mut surface = Display::open(bare_linux::PresentMode::Vsync)?;
        let size = surface.size();
        let (mut input, _keymap) = open_input(size)?;
        // Held for the whole run: dropping it puts the console back as it was.
        let _console = mute_console();

        // Built at the display's size, not a window's — which is the whole reason
        // the tree is not constructed until a backend knows how big it is.
        let mut app = setup.build(size, surface.scale_factor());
        if keyboard {
            app.focus_first_field();
        }
        eprintln!(
            "\nTap a field for the on-screen keyboard. Tab moves, Enter applies, F2 theme,\n\
             F12 screenshot, Escape puts the keyboard away and then quits\n"
        );

        // Refreshed by `wait` whenever a device is opened or closed, which is
        // why this is a `Waits` and not a list built once.
        let mut waits = Waits::new(&input);

        // The first frame, before anything is allowed to block.
        //
        // `poll` below waits on input and on whatever the tree says it wants
        // waking for. With nothing focused there is no caret, so nothing wants
        // waking, so the wait is the whole run — and a loop that waits before it
        // draws puts a mode on the display and then shows black until somebody
        // presses a key. It looks intermittent because launching from a shell
        // often leaves the Enter key's release in the evdev queue, which wakes the
        // poll and hides the bug.
        //
        // Drawing here rather than relying on something being focused is the
        // difference between a panel that starts and a panel that usually starts.
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
            let now = || started.elapsed().as_millis() as u64;

            // Blocks until input arrives or the caret owes a blink. With nothing
            // focused there is no timeout at all and the process uses no CPU.
            let timeout = poll_timeout(app.ui.next_wake_ms(), now(), deadline);
            waits.wait(&mut input, timeout.as_ref())?;

            events.clear();
            input.poll(&mut events);
            if !shortcuts(&mut app, &events, &mut shoot) {
                break;
            }
            app.ui.handle(&events);
            app.ui.tick(now());
            app.handle(now());

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
                // Not out from under the delete confirmation, which is a question
                // that deserves an answer — the same guard the window backend
                // applies.
                // The keyboard first, for the same reason as the window arm.
                KeyCode::Escape if app.keyboard_open() => app.dismiss_keyboard(),
                KeyCode::Escape if !app.is_confirming() => return false,
                KeyCode::ArrowUp if !app.is_confirming() => app.move_selection(-1),
                KeyCode::ArrowDown if !app.is_confirming() => app.move_selection(1),
                KeyCode::F2 => app.on_message(Message::NextTheme),
                KeyCode::F12 => *shoot = true,
                _ => {}
            }
        }
        true
    }
}
