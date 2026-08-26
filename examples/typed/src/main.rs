//! `hello` a third time — from a form file the **compiler** checks.
//!
//! ```text
//! cargo run -p typed
//! cargo run -p typed -- --snapshot out.ppm
//! ```
//!
//! Three versions of one twenty-line application now, and reading them in order
//! is the point:
//!
//! | | |
//! |---|---|
//! | [`examples/hello`](../../hello) | The tree, written in Rust. |
//! | [`examples/designed`](../../designed) | The same tree, read from `hello.dform` at load. Names are strings; a typo is a `None`. |
//! | this one | The same file again, turned into a struct and an enum at **build** time. A typo is a compile error. |
//!
//! All three draw the same pixels, and not as a figure of speech:
//!
//! ```text
//! cargo run -p hello    -- --snapshot a.ppm
//! cargo run -p designed -- --snapshot b.ppm
//! cargo run -p typed    -- --snapshot c.ppm
//! cmp a.ppm b.ppm && cmp b.ppm c.ppm    # silent
//! ```
//!
//! # What the generator gives you
//!
//! [`build.rs`](../build.rs) is four lines. Out of it comes:
//!
//! - `struct Hello`, with a `NodeId` field per **named** node in the form —
//!   `card`, `who`, `greeting`. Rename one in the designer and this file stops
//!   compiling, naming the field that is gone.
//! - `enum HelloMessage`, with a variant per message the form emits, carrying
//!   whatever that widget's payload is. Add one and every `match` stops being
//!   exhaustive.
//! - `Hello::build`, which calls the same engine [`designed`](../../designed)
//!   calls. There is one implementation of building a form; this is a typed door
//!   onto it.
//!
//! The `Wiring` impl that `designed` writes by hand — the `match` on a name that
//! the compiler cannot check — is generated here, with one arm per name the form
//! actually uses. A name the form uses and the application does not answer is
//! not an error at load; it is impossible.
//!
//! # What it costs
//!
//! **The form is baked in at build time.** That is what makes the names
//! checkable, and it is the one thing [`designed`](../../designed) can do that
//! this cannot: read the form from a file at run time and swap it without a
//! rebuild. If the screens on a panel are meant to be updated by copying files,
//! the untyped path is the one you want, and it is not a lesser one.

use std::time::{Duration, Instant};

use denise::{DamageTracker, ElementState, Frame, InputEvent, KeyCode, Rect, Size};
use denise_ui::Ui;
use denise_ui::widgets::{Label, TextInput};
use denise_winit::{DeniseApp, WindowConfig, run_with};

// `struct Hello`, `enum HelloMessage`, and `HELLO_SOURCE`. Generated from
// `forms/hello.dform` by `build.rs`; open
// `target/debug/build/typed-*/out/hello.rs` to read it.
include!(concat!(env!("OUT_DIR"), "/hello.rs"));

struct Typed {
    ui: Ui<HelloMessage>,
    /// The form's named nodes, as fields. Not looked up — *given*.
    form: Hello,
    started: Instant,
    exit: bool,
}

impl Typed {
    fn new(size: Size, _scale: f32) -> Self {
        let designed = Hello::form();
        let mut ui: Ui<HelloMessage> = Ui::new(size, designed.theme());
        let root = ui.root();

        // One call. No `Wiring` to write, no names to spell twice, and no
        // `.expect("the form names a field `who`")` — if the form did not name
        // it, this file would not have compiled.
        let form = Hello::build(&mut ui, root).expect("the form is checked in CI and baked in");

        Self {
            ui,
            form,
            started: Instant::now(),
            exit: false,
        }
    }

    /// The only piece of application logic, and it is `hello`'s, unchanged.
    fn greet(&mut self) {
        let name = self
            .ui
            .widget::<TextInput<HelloMessage>>(self.form.who)
            .map(|field| field.text().trim().to_string())
            .unwrap_or_default();
        let greeting = if name.is_empty() {
            "Hello, whoever you are.".to_string()
        } else {
            format!("Hello, {name}.")
        };
        if let Some(label) = self.ui.widget_mut::<Label>(self.form.greeting) {
            label.set_text(greeting);
        }
    }
}

impl DeniseApp for Typed {
    fn update(&mut self, events: &[InputEvent], damage: &mut DamageTracker) {
        for event in events {
            if let InputEvent::Key {
                code: KeyCode::Escape,
                state: ElementState::Down,
                ..
            } = event
            {
                self.exit = true;
            }
        }
        self.ui.handle(events);
        self.ui.tick(self.started.elapsed().as_millis() as u64);

        // The `match` the whole feature is for. Add `on-press=cancel` to the
        // form, rebuild, and this stops compiling until it is handled — which is
        // what a string-keyed resolver can never do.
        let messages: Vec<HelloMessage> = self.ui.drain_messages().collect();
        for message in messages {
            match message {
                HelloMessage::Greet => self.greet(),
            }
        }

        if self.ui.needs_paint() {
            let pending = self.ui.pending_damage();
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
        self.ui.paint(frame);
        self.ui.presented();
    }

    fn next_frame_in(&self) -> Option<Duration> {
        self.ui.next_wake_ms().map(|_| Duration::from_millis(16))
    }

    fn exit_requested(&self) -> bool {
        self.exit
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("--snapshot") {
        let path = args.next().unwrap_or_else(|| "typed.ppm".into());
        return snapshot(&path).map_err(Into::into);
    }

    // The form says how big it is and what it is called, so there is no second
    // number here to keep in step with it.
    let designed = Hello::form();
    run_with(
        WindowConfig {
            title: format!("Denise — {}", designed.title()),
            size: designed.size(),
            ..WindowConfig::default()
        },
        Typed::new,
    )?;
    Ok(())
}

/// One frame into a PPM, for the comparison in the module docs.
fn snapshot(path: &str) -> std::io::Result<()> {
    use std::io::Write as _;

    let size = Hello::form().size();
    let mut app = Typed::new(size, 1.0);
    // The same one line `hello` and `designed` do before their snapshots, so
    // the three pictures are comparable.
    app.greet();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generated_fields_are_the_names_the_form_gives() {
        let app = Typed::new(Size::new(460, 260), 1.0);
        // Three distinct nodes, which is the form's three `name=`s. If one were
        // renamed, this file would not have compiled at all.
        assert_ne!(app.form.who, app.form.greeting);
        assert_ne!(app.form.card, app.form.who);
    }

    #[test]
    fn typing_a_name_and_greeting_writes_the_label() {
        let mut app = Typed::new(Size::new(460, 260), 1.0);
        app.ui
            .widget_mut::<TextInput<HelloMessage>>(app.form.who)
            .expect("the field")
            .set_text(String::from("Ada"));
        app.greet();
        assert_eq!(
            app.ui.widget::<Label>(app.form.greeting).map(Label::text),
            Some("Hello, Ada."),
        );
    }
}
