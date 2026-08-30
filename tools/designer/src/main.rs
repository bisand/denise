//! A visual form designer for DeniseUI.
//!
//! ```text
//! denise-designer                      # an empty form
//! denise-designer forms/reference.dform
//! denise-designer --snapshot out.ppm forms/reference.dform
//! ```
//!
//! # It is a Denise application
//!
//! Not Tauri, not egui, not a web page. The canvas draws the form with **the same
//! code that will draw it on the panel** — the same widgets, the same rasteriser,
//! the same theme roles — so what is on screen here is what ships, to the pixel,
//! rather than an approximation that has to be kept in step.
//!
//! It also makes the designer the first real application written on this toolkit,
//! which is worth more than the convenience: what it lacks turns up here first.
//!
//! # The canvas is not a second `Ui`
//!
//! The form is built by [`denise_forms`] into a subtree of the designer's own
//! tree, inside a scrolling viewport. One tree, one event loop, one damage
//! tracker. What keeps the form from *behaving* while it is being designed is a
//! **scrim**: an invisible `Panel` with `backdrop` set, over the form and above
//! it, which absorbs every press and leaves the focus where it was. Preview mode
//! is hiding it, and that is the whole of preview mode.
//!
//! # What is here, and what is not
//!
//! A form opens, draws, selects — one at a time or by rubber band — moves,
//! resizes, reparents by dropping a node on a panel, reorders front to back,
//! aligns, sizes, spaces, groups, ungroups, copies, pastes, deletes and undoes;
//! a new one is asked what kind it is and writes only what is not a default,
//! and the form's own properties are the pane when nothing is selected; its properties are edited in the right pane; a widget is
//! dragged out of the palette or drawn on the canvas; and the outline shows the
//! tree as a tree, with folding, renaming, reparenting, and an eye that hides a
//! node here without the file learning of it. That is a form built from nothing.
//!
//! F5 runs it: the scrim goes, the events become the form's, and the strip along
//! the bottom names every message it fires. *Tab order* numbers every place Tab
//! can land, in the order it will land there, and clicking them re-sequences the
//! file — Delphi's mode, and the numbers come from the tree's own `tab_stops` so
//! they are the order the form will really have.
//!
//! The file underneath is watched, because the other editor is a text editor and
//! that was the point of a text format: saving renames a temporary file so it
//! never reads half a form, and a file written by something else is read again —
//! silently, keeping the selection by name, or with one question when there is
//! unsaved work to lose. See [`watch`].
//!
//! It draws itself in whatever face the machine has, falling back to the built-in
//! bitmap on a board with none — one call, since #130 made every unnamed
//! `TextStyle` a redirection through the tree's default face.
//!
//! The chrome is drawn at the **display's** scale ([`scale`]) and the canvas at
//! its own magnification ([`zoom`]) — two multiplications that look alike and
//! are not. The form's numbers never move: `width 800` means 800 at 25% and at
//! 400%, and a drag of one form pixel writes one.
//!
//! The palette is six shelves rather than a flat list, and resting on a row says
//! what the widget is — both declared by the widget through `Describe`, so a
//! twenty-sixth appears here filed and described without this crate changing.

mod app;
mod arrange;
mod canvas;
mod clipboard;
mod document;
mod history;
mod inspector;
mod outline;
mod scale;
mod settings;
mod text;
mod watch;
mod zoom;

use std::time::{Duration, Instant};

use denise::{DamageTracker, ElementState, Frame, InputEvent, KeyCode, Rect, Size};
use denise_winit::{DeniseApp, WindowConfig, run_with};

use app::{Designer, Message};
use document::Document;
use settings::Settings;
use zoom::Zoom;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut snapshot: Option<String> = None;
    let mut path: Option<String> = None;
    // Review aids for `--snapshot`, which has no pointer to select or drag with.
    let mut select: Option<String> = None;
    let mut drag: Option<(i32, i32)> = None;
    let mut carry: Option<(String, i32, i32)> = None;
    let mut preview = false;
    let mut band: Option<denise::Rect> = None;
    let mut new_form = false;
    let mut tab_order = false;
    let mut font: Option<String> = None;
    let mut scale: f32 = 1.0;
    let mut zoom: Option<String> = None;
    let mut hover: Option<String> = None;
    let mut clash: Option<String> = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--snapshot" => snapshot = rest.next().cloned().or(Some("designer.ppm".into())),
            "--select" => select = rest.next().cloned(),
            "--drag" => {
                drag = rest.next().and_then(|value| {
                    let (dx, dy) = value.split_once(',')?;
                    Some((dx.trim().parse().ok()?, dy.trim().parse().ok()?))
                });
            }
            "--preview" => preview = true,
            "--new" => new_form = true,
            "--tab-order" => tab_order = true,
            "--font" => font = rest.next().cloned(),
            "--zoom" => zoom = rest.next().cloned(),
            "--scale" => {
                scale = rest
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(1.0)
            }
            "--hover" => hover = rest.next().cloned(),
            "--clash" => clash = rest.next().cloned(),
            "--band" => {
                band = rest.next().and_then(|value| {
                    let mut parts = value.split(',').map(|part| part.trim().parse::<i32>());
                    let mut next = || parts.next().and_then(Result::ok);
                    Some(denise::Rect::new(next()?, next()?, next()?, next()?))
                });
            }
            "--carry" => {
                carry = rest.next().and_then(|value| {
                    let mut parts = value.split(',');
                    let kind = parts.next()?.trim().to_string();
                    let x = parts
                        .next()
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(120);
                    let y = parts
                        .next()
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(120);
                    Some((kind, x, y))
                });
            }
            "-h" | "--help" => {
                println!(
                    "denise-designer [form.dform]\n\n\
                     A visual form designer for DeniseUI.\n\n\
                     \x20 --snapshot <out.ppm>   draw one frame and exit, with no window\n\
                     \x20 --select <name>        snapshot: select this node first\n\
                     \x20 --drag <dx>,<dy>       snapshot: and drag it, so the guides show\n\
                     \x20 --carry <kind>[,x,y]   snapshot: hold this widget over the form\n\
                     \x20 --preview              snapshot: run the form rather than draw it\n\
                     \x20 --band <x,y,w,h>       snapshot: draw a rubber band over the form\n\
                     \x20 --new                  snapshot: put the new-form sheet up\n\
                     \x20 --tab-order            snapshot: number the form's tab stops\n\
                     \x20 --font <path.ttf>      draw in this face rather than the one found\n\
                     \x20 --scale <factor>       snapshot: draw the chrome at this display scale\n\
                     \x20 --zoom <percent|fit>   draw the form at this magnification\n\
                     \x20 --hover <kind>         snapshot: rest the pointer on this palette row\n\
                     \x20 --clash <other.dform>  snapshot: the file-changed sheet, against this version"
                );
                return Ok(());
            }
            "-V" | "--version" => {
                println!("denise-designer {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other => path = Some(other.to_string()),
        }
    }

    let settings = Settings::load();
    let document = match &path {
        Some(path) => Document::open(path)?,
        None => Document::blank(),
    };

    // One frame into a file, with no window and no event loop — the same
    // affordance every example here has, and the way this one's own layout gets
    // reviewed over SSH or diffed in a pull request.
    if let Some(out) = snapshot {
        // `--scale` is what a window would have reported. The surface grows
        // with it, exactly as a real one does: the window is a fixed amount of
        // desk and a denser display puts more pixels behind it.
        let physical = |logical: u32| ((logical as f32 * scale) + 0.5) as u32;
        let size = Size::new(physical(settings.width), physical(settings.height));
        let mut designer = Designer::new(size, scale, settings, document);
        use_zoom(&mut designer, zoom.as_deref());
        // A snapshot keeps the built-in face unless one is named. Its whole
        // value is being comparable — between two runs, between two machines,
        // between the two sides of a pull request — and a face picked up from
        // whatever is installed is none of those. `--font` is how a committed
        // screenshot pins one. The same reason `scripts/screenshot-browser.sh`
        // renders in a container.
        if let Some(path) = &font {
            use_font(&mut designer, Some(path));
        }
        if let Some(name) = select
            && !designer.select_named(&name)
        {
            eprintln!("denise-designer: this form has no node called `{name}`");
        }
        if let Some((dx, dy)) = drag {
            designer.drag_selection(dx, dy);
        }
        if preview {
            designer.toggle_preview();
        }
        if let Some(rect) = band {
            designer.band_over(rect);
        }
        if new_form {
            designer.begin_new();
        }
        if tab_order {
            designer.toggle_tab_order();
        }
        if let Some(kind) = hover
            && !designer.hover_palette(&kind)
        {
            eprintln!("denise-designer: the palette has no row called `{kind}`");
        }
        if let Some(other) = clash {
            let theirs = std::fs::read_to_string(&other)?;
            if !designer.clash_over(&theirs) {
                eprintln!("denise-designer: {other} is not a form file");
            }
        }
        if let Some((kind, x, y)) = carry
            && !designer.carry(&kind, denise::Point::new(x, y))
        {
            eprintln!("denise-designer: there is no widget called `{kind}`");
        }
        return write_snapshot(&mut designer, size, &out).map_err(Into::into);
    }

    let title = format!(
        "Denise designer — {}",
        path.as_deref().unwrap_or("Untitled")
    );
    run_with(
        WindowConfig {
            title,
            size: Size::new(settings.width, settings.height),
            resizable: true,
            ..WindowConfig::default()
        },
        move |size, scale| {
            let mut main = Main::new(size, scale, settings, document);
            // On the way up, so every pane is drawn in it from the first frame.
            use_font(&mut main.designer, font.as_deref());
            use_zoom(&mut main.designer, zoom.as_deref());
            main
        },
    )?;
    Ok(())
}

/// Draws the designer in a real face, if this machine has one.
///
/// The designer is a desktop tool and should look like the desktop it is on.
/// `system_font::load` walks the places systems keep fonts and prefers a regular
/// upright sans; on a board that has none — a Pi with no fonts installed — it
/// finds nothing and the built-in 5x7 bitmap stays, which is the right answer
/// there rather than a fallback to apologise for.
///
/// One call reaches every widget, already built or not, because every
/// `TextStyle` in the workspace names `FontId::DEFAULT` and that is a
/// redirection. Before #130 this needed a `TextStyle` threaded through every
/// widget the designer constructs, which is why it was never done.
/// Sets the canvas's magnification from `--zoom`.
///
/// `fit` or a percentage. Anything else is refused out loud rather than quietly
/// taken as 100%: a snapshot drawn at the wrong magnification is a picture that
/// looks right and is not.
fn use_zoom(designer: &mut Designer, requested: Option<&str>) {
    let Some(text) = requested else {
        return;
    };
    let asked = text.trim().trim_end_matches('%');
    if asked.eq_ignore_ascii_case("fit") {
        designer.zoom_to_fit();
        return;
    }
    match asked.parse::<u16>() {
        Ok(percent) => designer.set_zoom(Zoom::at(percent)),
        Err(_) => eprintln!("denise-designer: --zoom takes a percentage or `fit`, not `{text}`"),
    }
}

fn use_font(designer: &mut Designer, requested: Option<&str>) {
    let Some((name, source)) = system_font::load(requested) else {
        return;
    };
    let id = designer.ui.add_font(source);
    designer.ui.set_default_font(id);
    eprintln!("drawing in {name}");
}

struct Main {
    designer: Designer,
    started: Instant,
}

impl Main {
    fn new(size: Size, scale: f32, settings: Settings, document: Document) -> Self {
        Self {
            designer: Designer::new(size, scale, settings, document),
            started: Instant::now(),
        }
    }
}

impl DeniseApp for Main {
    fn title(&self) -> Option<&str> {
        Some(self.designer.window_title())
    }

    fn update(&mut self, events: &[InputEvent], damage: &mut DamageTracker) {
        for event in events {
            // The window's own size is what gets remembered, so it is read from
            // the event rather than from the tree, which has already scaled it.
            if let InputEvent::SurfaceResized { size, .. } = event {
                self.designer.remember_size(*size);
            }
            if let InputEvent::Key {
                code: KeyCode::Escape,
                state: ElementState::Down,
                ..
            } = event
            {
                self.designer.request_exit();
            }
        }

        // Design mode reads them first and keeps what is its own — a press on the
        // canvas is a selection or a drag, and the tree must never see it. What
        // comes back is everything else, so the toolbar and the panes go on
        // working while the form under design does not.
        let rest = self.designer.input(events);
        // First, and unedited: see `Designer::keyboard_input`.
        self.designer.keyboard_input(&rest);
        self.designer.ui.handle(&rest);

        let now = self.started.elapsed().as_millis() as u64;
        self.designer.ui.tick(now);
        self.designer.keyboard_turn(now);

        // Drained until it stops rather than once: a tap on the on-screen
        // keyboard is answered by feeding events straight back into the tree,
        // and whatever *those* produce belongs to the same frame as the tap.
        // Bounded, so a message that produced itself would cost a frame rather
        // than the application.
        for _ in 0..8 {
            let messages: Vec<Message> = self.designer.ui.drain_messages().collect();
            if messages.is_empty() {
                break;
            }
            for message in messages {
                self.designer.handle(message);
            }
        }

        // The other editor is a text editor, and it may have just saved. Before
        // the inspector is read, so a reload does not land on top of a commit
        // from the tree it replaced.
        self.designer.check_file();

        // Last, so a keystroke that has just reached a field is applied in the
        // same frame it was typed in. Nothing in the inspector emits a message;
        // see `Designer::poll`. There is no inspector while previewing, and no
        // palette filter either.
        if self.designer.previewing() {
            // A tab picked on the running form: `Tabs` owns the selection and
            // the host owns the page, and while previewing this is the host.
            self.designer.follow_previewed_tabs();
        } else {
            self.designer.poll();
        }

        if self.designer.ui.needs_paint() {
            let pending = self.designer.ui.pending_damage();
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
        self.designer.ui.paint(frame);
        self.designer.ui.presented();
    }

    fn next_frame_in(&self) -> Option<Duration> {
        // A designer is idle almost all the time: nothing animates unless a form
        // under design has a spinner in it, and then only that — and a file with
        // an editor open on it is worth a look either way.
        self.designer.next_frame_in()
    }

    fn exit_requested(&self) -> bool {
        if self.designer.exit_requested() {
            // The one place the settings are written: on the way out, once.
            self.designer.settings().save();
            return true;
        }
        false
    }
}

/// Draws one frame into a PPM.
fn write_snapshot(designer: &mut Designer, size: Size, path: &str) -> std::io::Result<()> {
    use std::io::Write as _;

    let mut pixels = vec![0u32; (size.width as usize) * (size.height as usize)];
    {
        let mut frame = Frame::new(
            &mut pixels,
            size,
            size.width,
            denise::PixelFormat::Xrgb8888,
            denise::BufferAge::Undefined,
        )
        .expect("a frame the size of its own buffer");
        designer.ui.paint(&mut frame);
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
