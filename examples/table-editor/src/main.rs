//! A record editor: a scrolling grid, an edit form, validation and a modal.
//!
//! ```text
//! cargo run -p table-editor
//! cargo run -p table-editor -- --font /usr/share/fonts/truetype/dejavu/DejaVuSans.ttf
//! cargo run -p table-editor -- --snapshot shot.ppm
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
use denise_text::TrueTypeSource;
use denise_ui::TextStyle;

const WINDOW: Size = Size::new(1000, 470);

/// Directories systems keep fonts in.
///
/// Directories, not files. The first version listed full paths and missed Alpine,
/// which puts DejaVu in `/usr/share/fonts/dejavu/` where Debian uses
/// `/usr/share/fonts/truetype/dejavu/` and Arch uses `/usr/share/fonts/TTF/`.
/// Guessing one more path would have been wrong on the next distribution too, so
/// this looks instead.
const FONT_DIRS: &[&str] = &[
    "/usr/share/fonts",
    "/usr/local/share/fonts",
    "/System/Library/Fonts",
    "/System/Library/Fonts/Supplemental",
    "/Library/Fonts",
    "C:\\Windows\\Fonts",
];

/// Faces worth having, best first.
///
/// A regular upright sans, because that is what a grid of records wants. Nothing
/// here is required — it is the order of preference among whatever turns up.
const PREFERRED: &[&str] = &[
    "DejaVuSans.ttf",
    "LiberationSans-Regular.ttf",
    "NotoSans-Regular.ttf",
    "Inter-Regular.ttf",
    "segoeui.ttf",
    "Arial.ttf",
    "Helvetica.ttc",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut path = "people.csv".to_string();
    let mut font: Option<String> = None;
    let mut snapshot: Option<String> = None;
    // Kiosk builds have no window to close, so a run length is the way out. Zero
    // means "until Escape".
    let mut seconds: u64 = 0;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--font" => font = args.next(),
            "--snapshot" => snapshot = Some(args.next().unwrap_or_else(|| "table.ppm".into())),
            "--seconds" => seconds = args.next().and_then(|s| s.parse().ok()).unwrap_or(0),
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
        font: load_font(font.as_deref()),
    };

    if let Some(out) = snapshot {
        return write_snapshot(&mut setup.build(WINDOW), &out).map_err(Into::into);
    }
    backend::run(setup, seconds)
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
    pub fn build(self, size: Size) -> App {
        let mut app = App::new(
            size,
            self.path,
            self.table,
            TextStyle::built_in(16),
            TextStyle::built_in(24),
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

/// Loads the named font, or the best one this machine has.
fn load_font(requested: Option<&str>) -> Option<(String, Box<dyn denise_text::GlyphSource>)> {
    let path = match requested {
        Some(path) => std::path::PathBuf::from(path),
        None => match find_font() {
            Some(found) => found,
            None => {
                eprintln!("no font found under {}", FONT_DIRS.join(", "));
                return None;
            }
        },
    };

    let name = path.display().to_string();
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("{name}: {e}");
            return None;
        }
    };
    match TrueTypeSource::from_bytes(&name, &bytes) {
        Ok(source) => Some((name, Box::new(source))),
        // Says *why*, rather than falling back in silence: a font that is present
        // and unreadable is a different problem from one that is not there, and
        // only one of them is fixed by installing something.
        Err(why) => {
            eprintln!("{name}: {why}");
            None
        }
    }
}

/// The best face under [`FONT_DIRS`], or `None` if the machine has none.
fn find_font() -> Option<std::path::PathBuf> {
    let mut found = Vec::new();
    for dir in FONT_DIRS {
        collect(std::path::Path::new(dir), 0, &mut found);
    }
    pick(&found).cloned()
}

/// Every font file under `dir`, to a bounded depth.
///
/// Bounded because a font directory is somebody else's, and following it without
/// a limit is how a panel spends its startup walking a symlink loop.
fn collect(dir: &std::path::Path, depth: usize, out: &mut Vec<std::path::PathBuf>) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, depth + 1, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("ttf" | "otf" | "ttc")
        ) {
            out.push(path);
        }
    }
}

/// Chooses among the faces that were found.
///
/// Separated from the walking so it can be tested: the choosing is the part that
/// goes wrong, and picking `DejaVuSans-BoldOblique` over `DejaVuSans` is exactly
/// the sort of wrong that nobody notices until they see a screenshot.
fn pick(found: &[std::path::PathBuf]) -> Option<&std::path::PathBuf> {
    let name_of = |p: &std::path::PathBuf| {
        p.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
    };

    for wanted in PREFERRED {
        let wanted = wanted.to_ascii_lowercase();
        if let Some(hit) = found.iter().find(|p| name_of(p) == wanted) {
            return Some(hit);
        }
    }

    // Nothing preferred, so anything upright and proportional. A grid set in Bold
    // Italic Condensed is worse than one set in the bitmap font.
    found.iter().find(|p| {
        let name = name_of(p);
        !["bold", "italic", "oblique", "condensed", "mono", "light"]
            .iter()
            .any(|bad| name.contains(bad))
    })
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
    use super::{App, Message, Setup, WINDOW};
    use denise::{DamageTracker, ElementState, Frame, InputEvent, KeyCode, Rect};
    use denise_winit::{DeniseApp, WindowConfig, run as run_window};

    pub fn run(setup: Setup, _seconds: u64) -> Result<(), Box<dyn std::error::Error>> {
        // A window's size is the application's choice, so this one is `WINDOW`.
        let mut app = setup.build(WINDOW);
        // The window system already draws a pointer; the tree must not draw a
        // second one over it. On the kiosk backend there is nothing else drawing
        // one, so it keeps its own — the same tree, a different right answer.
        app.ui.show_cursor(false);
        run_window(
            WindowConfig {
                title: "Denise — table editor".into(),
                size: WINDOW,
                ..WindowConfig::default()
            },
            Editor(app),
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
                        _ => {}
                    }
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

        fn exit_requested(&self) -> bool {
            self.0.exit
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
    use std::os::fd::BorrowedFd;
    use std::time::{Duration, Instant};

    use super::{App, Message, Setup};
    use bare_linux::{Display, capture, mute_console, open_input, poll_timeout};
    use denise::{ElementState, InputEvent, InputSource, KeyCode, Surface};
    use rustix::event::{PollFd, PollFlags, poll};

    /// Where F12 writes. `/tmp` because a kiosk image is very often read-only
    /// everywhere else.
    const SHOT_PATH: &str = "/tmp/denise-table-editor.ppm";

    pub fn run(setup: Setup, seconds: u64) -> Result<(), Box<dyn std::error::Error>> {
        // Immediate rather than vsync: this panel is driven by a keypad and a
        // pointer, so latency matters more to it than tearing does.
        let mut surface = Display::open(bare_linux::PresentMode::Immediate)?;
        let size = surface.size();
        let (mut input, _keymap) = open_input(size)?;
        // Held for the whole run: dropping it puts the console back as it was.
        let _console = mute_console();

        // Built at the display's size, not a window's — which is the whole reason
        // the tree is not constructed until a backend knows how big it is.
        let mut app = setup.build(size);
        eprintln!("\nTab moves, Enter applies, F2 theme, F12 screenshot, Escape quits\n");

        // Built once: the device set does not change, and `poll` updates each
        // entry's revents in place.
        let raw_fds = input.raw_fds();
        let borrowed: Vec<BorrowedFd<'_>> = raw_fds
            .iter()
            // SAFETY: every descriptor belongs to `input`, which outlives this loop
            // and holds each device open until the process exits.
            .map(|&fd| unsafe { BorrowedFd::borrow_raw(fd) })
            .collect();
        let mut poll_fds: Vec<PollFd<'_>> = borrowed
            .iter()
            .map(|fd| PollFd::new(fd, PollFlags::IN))
            .collect();

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
            poll(&mut poll_fds, timeout.as_ref())?;

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
                KeyCode::Escape => return false,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    /// The bug this replaced: hardcoded full paths that knew Debian's layout and
    /// Arch's, and not Alpine's. Every one of these is the same face on a
    /// different distribution, and all three have to be found.
    #[test]
    fn dejavu_is_found_wherever_a_distribution_puts_it() {
        for dir in [
            "/usr/share/fonts/dejavu",          // Alpine
            "/usr/share/fonts/truetype/dejavu", // Debian, Raspberry Pi OS
            "/usr/share/fonts/TTF",             // Arch
        ] {
            let found = paths(&[
                &format!("{dir}/DejaVuSerif.ttf"),
                &format!("{dir}/DejaVuSans.ttf"),
            ]);
            assert_eq!(
                pick(&found).map(|p| p.file_name().unwrap().to_str().unwrap()),
                Some("DejaVuSans.ttf"),
                "not found under {dir}"
            );
        }
    }

    /// A directory of one face is mostly its variants, and the plain one is
    /// usually not first. Alpine's dejavu directory lists ExtraLight before Sans.
    #[test]
    fn the_plain_face_wins_over_its_variants() {
        let found = paths(&[
            "/usr/share/fonts/dejavu/DejaVuSans-ExtraLight.ttf",
            "/usr/share/fonts/dejavu/DejaVuSansCondensed-BoldOblique.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        ]);
        assert_eq!(
            pick(&found).map(|p| p.file_name().unwrap().to_str().unwrap()),
            Some("DejaVuSans.ttf")
        );
    }

    /// Preference order, not directory order: a machine with both should get the
    /// one that reads better in a data grid.
    #[test]
    fn preference_beats_whatever_was_listed_first() {
        let found = paths(&["/x/Arial.ttf", "/x/DejaVuSans.ttf"]);
        assert_eq!(
            pick(&found).map(|p| p.file_name().unwrap().to_str().unwrap()),
            Some("DejaVuSans.ttf")
        );
    }

    /// Nothing preferred is not nothing usable — but a face that is only
    /// available bold and italic is worse than the built-in bitmap font, so it is
    /// refused and the caller falls back.
    #[test]
    fn an_unknown_upright_face_is_taken_and_a_slanted_one_is_not() {
        assert_eq!(
            pick(&paths(&["/x/SomethingElse.ttf"])).map(|p| p.display().to_string()),
            Some("/x/SomethingElse.ttf".to_string())
        );
        assert_eq!(pick(&paths(&["/x/Whatever-BoldItalic.ttf"])), None);
        assert_eq!(pick(&[]), None);
    }
}
