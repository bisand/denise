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
//! This is the skeleton: the panes, a form opened and drawn, an outline that
//! selects, an inspector that reports, and a save that writes back what was read.
//! Dragging from the palette is [#91], moving things on the canvas is [#92], and
//! editing a property is [#93].
//!
//! [#91]: https://github.com/bisand/denise/issues/91
//! [#92]: https://github.com/bisand/denise/issues/92
//! [#93]: https://github.com/bisand/denise/issues/93

mod app;
mod canvas;
mod document;
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
            "-h" | "--help" => {
                println!(
                    "denise-designer [form.dform]\n\n\
                     A visual form designer for DeniseUI.\n\n\
                     \x20 --snapshot <out.ppm>   draw one frame and exit, with no window\n\
                     \x20 --select <name>        snapshot: select this node first\n\
                     \x20 --drag <dx>,<dy>       snapshot: and drag it, so the guides show"
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
        self.designer.ui.handle(&rest);
        self.designer
            .ui
            .tick(self.started.elapsed().as_millis() as u64);

        let messages: Vec<Message> = self.designer.ui.drain_messages().collect();
        for message in messages {
            self.designer.handle(message);
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
