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
//! `app.rs` never learns where it is running. This file has the two backends —
//! a window on macOS, Windows and desktop Linux, and the display itself on a
//! Raspberry Pi with `--drm` — and each is about forty lines. That split is the
//! library's whole claim, made small enough to check.

mod app;
mod table;

use app::{App, Message};
use denise::Size;
use denise_text::TrueTypeSource;
use denise_ui::TextStyle;

const WINDOW: Size = Size::new(1000, 470);

/// Where a body-text-sized face usually lives, per platform.
///
/// A guess, and a cheap one: if it is wrong the editor says so and draws with the
/// built-in font, which is a worse-looking panel rather than no panel. A kiosk
/// image would ship its font next to the binary and pass `--font`.
const SYSTEM_FONTS: &[&str] = &[
    // macOS
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
    // Debian, Raspberry Pi OS
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    // Windows
    "C:\\Windows\\Fonts\\segoeui.ttf",
    "C:\\Windows\\Fonts\\arial.ttf",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut path = "people.csv".to_string();
    let mut font: Option<String> = None;
    let mut snapshot: Option<String> = None;
    let mut drm = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--font" => font = args.next(),
            "--snapshot" => snapshot = Some(args.next().unwrap_or_else(|| "table.ppm".into())),
            "--drm" => drm = true,
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

    let mut ui_font = None;
    let mut app = App::new(
        WINDOW,
        path,
        table,
        TextStyle::built_in(16),
        TextStyle::built_in(24),
    );

    // The font is registered *after* the tree is built, because registering one
    // damages the whole surface — everything on screen may change width — and
    // doing it once at startup is the cheapest possible time for that.
    if let Some((name, source)) = load_font(font.as_deref()) {
        let id = app.ui.add_font(source);
        eprintln!("using {name}");
        ui_font = Some(id);
    } else {
        eprintln!("no TrueType font found; using the built-in 8x8 bitmap font");
    }
    if let Some(id) = ui_font {
        app.set_font(id);
    }

    if let Some(path) = snapshot {
        return write_snapshot(&mut app, &path).map_err(Into::into);
    }
    if drm {
        eprintln!(
            "the DRM backend is not wired into this example yet; \
             `examples/panel` is the one that drives a display directly"
        );
    }
    window_backend::run(app)
}

/// Loads the named font, or the first system one that exists.
fn load_font(requested: Option<&str>) -> Option<(String, Box<dyn denise_text::GlyphSource>)> {
    let candidates: Vec<&str> = match requested {
        Some(path) => vec![path],
        None => SYSTEM_FONTS.to_vec(),
    };
    for candidate in candidates {
        let Ok(bytes) = std::fs::read(candidate) else {
            continue;
        };
        match TrueTypeSource::from_bytes(candidate, &bytes) {
            Ok(source) => return Some((candidate.to_string(), Box::new(source))),
            // Says *why*, rather than silently falling back: a font that is
            // present and unreadable is a different problem from one that is not
            // there, and only one of them is fixed by installing something.
            Err(why) => eprintln!("{candidate}: {why}"),
        }
    }
    None
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

/// A window, on any desktop. Forty lines, all of them plumbing.
mod window_backend {
    use super::{App, Message, WINDOW};
    use denise::{DamageTracker, ElementState, Frame, InputEvent, KeyCode, Rect};
    use denise_winit::{DeniseApp, WindowConfig, run as run_window};

    pub fn run(app: App) -> Result<(), Box<dyn std::error::Error>> {
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
