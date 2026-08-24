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
//! A form opens, draws, selects, moves, resizes, deletes and undoes; its
//! properties are edited in the right pane; a widget is dragged out of the
//! palette or drawn on the canvas; and the outline shows the tree as a tree,
//! with folding, renaming, reparenting, and an eye that hides a node here
//! without the file learning of it. That is a form built from nothing.
//!
//! F5 runs it: the scrim goes, the events become the form's, and the strip along
//! the bottom names every message it fires.
//!
//! The palette is a flat list of names, because the registry carries a name and
//! a property list and nothing that says what a widget *is* — grouping it and
//! giving each row a tooltip is [#126].
//!
//! [#126]: https://github.com/bisand/denise/issues/126

mod app;
mod canvas;
mod document;
mod history;
mod inspector;
mod outline;
mod settings;

use std::time::{Duration, Instant};

use denise::{DamageTracker, ElementState, Frame, InputEvent, KeyCode, Rect, Size};
use denise_winit::{DeniseApp, WindowConfig, run_with};

use app::{Designer, Message};
use document::Document;
use settings::Settings;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut snapshot: Option<String> = None;
    let mut path: Option<String> = None;
    // Review aids for `--snapshot`, which has no pointer to select or drag with.
    let mut select: Option<String> = None;
    let mut drag: Option<(i32, i32)> = None;
    let mut carry: Option<(String, i32, i32)> = None;
    let mut preview = false;
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
                     \x20 --preview              snapshot: run the form rather than draw it"
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
        let size = Size::new(settings.width, settings.height);
        let mut designer = Designer::new(size, 1.0, settings, document);
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
        move |size, scale| Main::new(size, scale, settings, document),
    )?;
    Ok(())
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

        // Last, so a keystroke that has just reached a field is applied in the
        // same frame it was typed in. Nothing in the inspector emits a message;
        // see `Designer::poll`. There is no inspector while previewing, and no
        // palette filter either.
        if !self.designer.previewing() {
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
        // under design has a spinner in it, and then only that.
        self.designer
            .ui
            .next_wake_ms()
            .map(|_| Duration::from_millis(16))
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
