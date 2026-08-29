//! The `hello` example again, this time **built from a file**.
//!
//! ```text
//! cargo run -p designed
//! ```
//!
//! It is the same application: type a name, press Greet, read the greeting. The
//! difference is where the tree comes from. [`examples/hello`](../../hello)
//! writes twenty lines of `ui.add(...)`; this one reads
//! [`forms/hello.dform`](../../../forms/hello.dform), which is text a person
//! edits, `git diff` reads, and the [designer](../../../tools/designer) draws.
//!
//! Read the two side by side. What they have in common is the interesting part:
//! the same message enum, the same `update`, the same damage, the same event
//! loop. **A form replaces the tree-building and nothing else.**
//!
//! They do not merely look alike. They are the same pixels:
//!
//! ```text
//! cargo run -p hello    -- --snapshot a.ppm
//! cargo run -p designed -- --snapshot b.ppm
//! cmp a.ppm b.ppm       # silent
//! ```
//!
//! # The three things a form cannot supply
//!
//! A `.dform` file holds widgets and their initial state. It holds no code, so
//! there are three things the application still has to say, and
//! [`Wiring`] is where it says them:
//!
//! 1. **Its own message type.** The file says `on-press=greet`; only this crate
//!    knows that `greet` means [`Message::Greet`]. The mapping is a `match` on a
//!    string, which the compiler cannot check — so a name the form uses and the
//!    application does not know is an *error at load*, with the name in it,
//!    rather than a button that quietly does nothing.
//! 2. **Its pictures.** `hello.dform` names none, so [`Wiring::asset`] is left at
//!    its default here. The reference form names five; `denise-forms check` is
//!    what tells you when one has moved.
//! 3. **What the widgets are called.** The file's `name=who` becomes
//!    [`Built::node`]`("who")`, which is the [`NodeId`] to read the field with.
//!    That is the one place a typo in a name shows up as `None` rather than as a
//!    compile error, and [#101] is the issue for generating the names instead.
//!
//! # Baked in, not read at runtime
//!
//! The form is `include_str!`'d. A kiosk image with no writable filesystem and no
//! path to read from still gets its layout, and the binary cannot start without
//! one — the form is checked at *build* time by being a string literal and at
//! *load* time by [`Form::parse`]. Reading it with `std::fs::read_to_string`
//! instead is one line, and is what you want when the form is meant to be
//! swapped without a rebuild — with one thing to change besides: a form read at
//! runtime came from somewhere, so parse it with
//! [`Form::parse_within`](denise_forms::Form::parse_within) and a deadline
//! rather than [`Form::parse`]. Baked in, this file is not at risk: it was read
//! at build time from the repository.
//!
//! [#101]: https://github.com/bisand/denise/issues/101

#[cfg(not(all(feature = "kiosk", target_os = "linux")))]
use std::time::Duration;
use std::time::Instant;

use denise::Size;
#[cfg(not(all(feature = "kiosk", target_os = "linux")))]
use denise::{DamageTracker, ElementState, Frame, InputEvent, KeyCode, Rect};
use denise_forms::{Built, Form, Handler, Payload, Wiring};
use denise_ui::widgets::{Label, TextInput};
use denise_ui::{NodeId, Ui};
#[cfg(not(all(feature = "kiosk", target_os = "linux")))]
use denise_winit::{DeniseApp, WindowConfig, run_with};

/// The form, as text, compiled into the binary. See the module docs.
const FORM: &str = include_str!("../../../forms/hello.dform");

/// What the widgets send back.
///
/// Exactly the enum `hello` has. A form does not change what a message is; it
/// changes who says which widget sends one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Message {
    Greet,
}

/// What this application supplies that the file cannot: its own message type.
///
/// One `match` on a name. `payload` says which *shape* the widget needs — a
/// button holds the message itself, a checkbox holds a `fn(bool) -> M` — and
/// answering with the wrong shape is an error at load rather than a widget that
/// misbehaves later.
struct Wires;

impl Wiring<Message> for Wires {
    fn message(&mut self, name: &str, payload: Payload) -> Option<Handler<Message>> {
        match (name, payload) {
            // The button's `on-press` and the field's `on-submit`, which the form
            // deliberately gives the same name: pressing Enter and pressing the
            // button are the same thing to somebody using it.
            ("greet", Payload::None) => Some(Handler::Plain(Message::Greet)),
            _ => None,
        }
    }
}

#[cfg(all(feature = "kiosk", target_os = "linux"))]
mod kiosk;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("--snapshot") {
        let path = args.next().unwrap_or_else(|| "designed.ppm".into());
        return snapshot(&path).map_err(Into::into);
    }

    #[cfg(all(feature = "kiosk", target_os = "linux"))]
    return kiosk::run();

    #[cfg(not(all(feature = "kiosk", target_os = "linux")))]
    {
        // The form says how big it is and what it is called, so the window does
        // not have to be told twice. Change `width` in the file and the window
        // changes; there is no second number in this file to keep in step.
        let form = Form::parse(FORM)?;
        run_with(
            WindowConfig {
                title: format!("Denise — {}", form.title()),
                size: form.size(),
                ..WindowConfig::default()
            },
            Designed::new,
        )?;
        Ok(())
    }
}

struct Designed {
    ui: Ui<Message>,
    /// The field to read the name out of, found by the name the *file* gave it.
    name: NodeId,
    /// The label to write the greeting into, likewise.
    greeting: NodeId,
    started: Instant,
    #[cfg(not(all(feature = "kiosk", target_os = "linux")))]
    exit: bool,
}

impl Designed {
    /// Everything that differs from `hello` is in this function.
    fn new(size: Size, _scale: f32) -> Self {
        let form = Form::parse(FORM).expect("the form is compiled in and was checked by CI");

        // The form was designed at `form.size()`; the surface is whatever the
        // machine gives — a 460x260 window here, a 1920x1080 panel there. What
        // to do about the difference is the **file's** decision rather than this
        // application's: `hello.dform` says nothing, which means `scaling=none`,
        // which means its own size in the middle. Change that one word in the
        // file to `proportional` and this application fills the panel, with no
        // line here changing.
        let fit = form.fit(size);

        // The theme comes from the file too — a form that says `theme=light` is
        // a light application, with nothing here to change — and it is scaled
        // here, once, because a widget's corners and its border are the theme's
        // numbers rather than the file's.
        let mut ui: Ui<Message> = Ui::new(size, form.theme().scaled(fit.uniform()));
        let root = ui.root();

        let stage = ui
            .add(
                root,
                denise_ui::widgets::Panel::filled(form.background()),
                fit.rect,
            )
            .expect("the root takes a child");

        // The whole of building the tree. Errors say which node and which
        // property, with a line and a column, because a form is a file somebody
        // typed and the message has to be usable by whoever typed it.
        let built: Built = form
            .build_fitted(&mut ui, stage, fit, &mut Wires)
            .expect("the form builds; `denise-forms check` says so in CI");

        // The named nodes, by the names in the file. Everything else the form
        // put on screen — the two headings, the button — this application never
        // needs to reach again, and so never names.
        let name = built.node("who").expect("the form names a field `who`");
        let greeting = built
            .node("greeting")
            .expect("the form names a label `greeting`");

        Self {
            ui,
            name,
            greeting,
            started: Instant::now(),
            #[cfg(not(all(feature = "kiosk", target_os = "linux")))]
            exit: false,
        }
    }

    /// The only piece of application logic, and it is `hello`'s, unchanged.
    fn greet(&mut self) {
        let name = self
            .ui
            .widget::<TextInput<Message>>(self.name)
            .map(|field| field.text().trim().to_string())
            .unwrap_or_default();

        let greeting = if name.is_empty() {
            "Hello, whoever you are.".to_string()
        } else {
            format!("Hello, {name}.")
        };

        if let Some(label) = self.ui.widget_mut::<Label>(self.greeting) {
            label.set_text(greeting);
        }
    }
}

#[cfg(not(all(feature = "kiosk", target_os = "linux")))]
impl DeniseApp for Designed {
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

        let messages: Vec<Message> = self.ui.drain_messages().collect();
        for message in messages {
            match message {
                Message::Greet => self.greet(),
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

    fn exit_requested(&self) -> bool {
        self.exit
    }

    fn next_frame_in(&self) -> Option<Duration> {
        let now = self.started.elapsed().as_millis() as u64;
        self.ui
            .next_wake_ms()
            .map(|wake| Duration::from_millis(wake.saturating_sub(now)))
    }
}

/// Draws one frame into a file, with no window and no event loop.
fn snapshot(path: &str) -> std::io::Result<()> {
    use std::io::Write as _;

    let size = Form::parse(FORM).expect("the form parses").size();
    let mut app = Designed::new(size, 1.0);
    app.greet();

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_this_binary_carries_is_a_form() {
        let form = Form::parse(FORM).expect("the compiled-in form parses");
        assert_eq!(form.title(), "Hello");
        assert_eq!(form.size(), Size::new(460, 260));
    }

    #[test]
    fn every_message_the_form_names_is_one_this_application_knows() {
        // The check that cannot be made at compile time, made at test time
        // instead: a form naming a message nobody wired fails to build, and it
        // fails *here* rather than in front of somebody.
        let form = Form::parse(FORM).expect("parses");
        let mut ui: Ui<Message> = Ui::new(Size::new(460, 260), denise::theme::DARK);
        let root = ui.root();
        form.build(&mut ui, root, &mut Wires)
            .expect("a name the form uses is one `Wires` answers");
    }

    #[test]
    fn the_names_this_application_reaches_for_are_names_the_form_gives() {
        let app = Designed::new(Size::new(460, 260), 1.0);
        // `new` would have panicked; this says which two it was reaching for.
        assert_ne!(app.name, app.greeting);
    }

    #[test]
    fn a_surface_bigger_than_the_form_centres_it_rather_than_stretching_it() {
        // A form file carries a design size, not a promise about the display.
        // On a panel twice the size the tree is the size the file says, in the
        // middle — because `hello.dform` says nothing about scaling, and saying
        // nothing means `scaling=none`. The file decides; this only obeys.
        let app = Designed::new(Size::new(920, 520), 1.0);
        let field = app.ui.bounds(app.name).expect("laid out");
        let small = Designed::new(Size::new(460, 260), 1.0);
        let same = small.ui.bounds(small.name).expect("laid out");

        assert_eq!(
            (field.width, field.height),
            (same.width, same.height),
            "the form was stretched"
        );
        assert_eq!(field.x - same.x, 230, "half the difference, on each side");
        assert_eq!(field.y - same.y, 130);
    }

    #[test]
    fn greeting_writes_what_was_typed_into_the_label_the_form_named() {
        let mut app = Designed::new(Size::new(460, 260), 1.0);
        if let Some(field) = app.ui.widget_mut::<TextInput<Message>>(app.name) {
            field.set_text("Ada");
        }
        app.greet();
        assert_eq!(
            app.ui
                .widget::<Label>(app.greeting)
                .map(|label| label.text().to_string()),
            Some(String::from("Hello, Ada."))
        );
    }
}
