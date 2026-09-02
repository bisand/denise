//! Two forms, read from files **at run time**, and one application answering
//! both.
//!
//! ```text
//! cargo run -p runtime
//! cargo run -p runtime -- --snapshot out.ppm [hello|settings]
//! ```
//!
//! [`designed`](../../designed) compiles its form in with `include_str!`; this
//! one reads its two from [`forms/`](../forms) when it needs them. Edit either
//! file — or copy a different one over it — and the next start shows the new
//! face with no rebuild. That is the untyped path from
//! [docs/forms.md](../../../docs/forms.md#baked-in-or-read-at-runtime), and
//! this is what it looks like with more than one screen.
//!
//! # What the test is
//!
//! A form is only ever layout and initial state. What it *does* is the
//! application's, said once in [`Wires`]: a `match` from every event name
//! either file uses to a function here. So the things worth checking, and
//! checked in the tests at the bottom, are:
//!
//! - both files load, which means every name they use is one [`Wires`]
//!   answers — a name it does not answer is an **error at load, naming the
//!   event**, and the last test shows that too;
//! - pressing *Settings* on one screen swaps in the other file, and *Back*
//!   swaps back;
//! - events carrying values arrive with them — a checkbox's `bool`, a slider's
//!   `f32` — and change state that lives in the application;
//! - that state outlives the screen it was set on: the greeting is still there
//!   after a trip to Settings and back, because the form is a face and the
//!   application is the thing.
//!
//! # And the designer
//!
//! Each form has a sidecar beside it, `hello.designer` and `settings.designer`,
//! naming this file and `handlers = Runtime`. Open either form in the designer,
//! select the button, and the arrow beside `on-press` lands on the method here;
//! type an event name this file does not answer and the designer draws it in
//! red before the load ever fails.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use denise::{DamageTracker, ElementState, Frame, InputEvent, KeyCode, Rect, Size};
use denise_forms::{Built, Form, Handler, Payload, Wiring};
use denise_ui::widgets::{Checkbox, Label, Panel, Slider, TextInput};
use denise_ui::{NodeId, Ui};
use denise_winit::{DeniseApp, WindowConfig, run_with};

/// How long a form file may take to parse before it is treated as hostile.
///
/// A form read at run time came from somewhere; see `Form::parse_within`.
const PATIENCE: Duration = Duration::from_millis(250);

/// The two screens, each a file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Screen {
    Hello,
    Settings,
}

impl Screen {
    /// The file the screen is read from.
    fn file(self) -> &'static str {
        match self {
            Self::Hello => "hello.dform",
            Self::Settings => "settings.dform",
        }
    }
}

/// What the widgets send back — from either form. One enum, because it is one
/// application; a form does not change what a message is, only which widget
/// sends it.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Message {
    Greet,
    Settings,
    Back,
    SetNotify(bool),
    SetVolume(f32),
}

/// What this application supplies that the files cannot: its own message
/// type. One `match`, answering every name *either* form uses, with the shape
/// each widget needs — a button holds the message itself, a checkbox a
/// `fn(bool) -> M`. A name not here fails the form at load, with the name in
/// the error.
struct Wires;

impl Wiring<Message> for Wires {
    fn message(&mut self, name: &str, payload: Payload) -> Option<Handler<Message>> {
        match (name, payload) {
            ("greet", Payload::None) => Some(Handler::Plain(Message::Greet)),
            ("settings", Payload::None) => Some(Handler::Plain(Message::Settings)),
            ("back", Payload::None) => Some(Handler::Plain(Message::Back)),
            ("set-notify", Payload::Bool) => Some(Handler::Bool(Message::SetNotify)),
            ("set-volume", Payload::Number) => Some(Handler::Number(Message::SetVolume)),
            _ => None,
        }
    }
}

/// Where the forms are read from: beside this crate, unless told otherwise.
///
/// A path rather than an `include_str!`, which is the whole point: the files
/// are read when a screen is shown, and the binary does not change when they
/// do.
fn forms_dir() -> PathBuf {
    std::env::var_os("RUNTIME_FORMS").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("forms"),
        PathBuf::from,
    )
}

/// Reads and parses one screen's file, now.
fn load(screen: Screen) -> Result<Form, String> {
    let path = forms_dir().join(screen.file());
    let text = std::fs::read_to_string(&path)
        .map_err(|why| format!("could not read {}: {why}", path.display()))?;
    Form::parse_within(&text, PATIENCE).map_err(|why| format!("{}: {why}", path.display()))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("--snapshot") {
        let path = args.next().unwrap_or_else(|| "runtime.ppm".into());
        let screen = match args.next().as_deref() {
            Some("settings") => Screen::Settings,
            _ => Screen::Hello,
        };
        return snapshot(&path, screen).map_err(Into::into);
    }

    // The first screen says how big the window is; both files are designed at
    // the same size, which is the one thing the two have to agree on.
    let form = load(Screen::Hello)?;
    run_with(
        WindowConfig {
            title: format!("Denise — {}", form.title()),
            size: form.size(),
            ..WindowConfig::default()
        },
        Runtime::new,
    )?;
    Ok(())
}

/// The application: the state the forms are faces of, and which face is on.
struct Runtime {
    ui: Ui<Message>,
    /// The surface the screen's size the machine gives.
    size: Size,
    /// Which file is on screen.
    screen: Screen,
    /// The node the screen is built under, replaced on every swap.
    stage: NodeId,
    /// The screen's named nodes, by the names its file gave them.
    built: Built,
    /// The state that outlives any one screen.
    greeting: String,
    notify: bool,
    volume: f32,
    started: Instant,
    exit: bool,
}

impl Runtime {
    fn new(size: Size, _scale: f32) -> Self {
        let form = load(Screen::Hello).expect("the first screen's file is beside this crate");
        let fit = form.fit(size);
        // The theme is the first file's, scaled once for this surface. Both
        // files here say `dark`; a screen that said otherwise would be drawn in
        // the first one's clothes, which is the honest limit of one `Ui`.
        let mut ui: Ui<Message> = Ui::new(size, form.theme().scaled(fit.uniform()));
        let root = ui.root();
        let stage = ui
            .add(root, Panel::filled(form.background()), fit.rect)
            .expect("the root takes a child");
        let built = form
            .build_fitted(&mut ui, stage, fit, &mut Wires)
            .expect("the first screen builds; CI checks every form file");
        let mut app = Self {
            ui,
            size,
            screen: Screen::Hello,
            stage,
            built,
            greeting: String::new(),
            notify: false,
            volume: 35.0,
            started: Instant::now(),
            exit: false,
        };
        app.dress();
        app
    }

    /// Swaps the screen for `screen`'s file, read now.
    ///
    /// The old subtree goes, the file is read and built under a fresh stage,
    /// and the application's state is put onto the new widgets — which is what
    /// makes the greeting survive a trip to Settings and back.
    fn show(&mut self, screen: Screen) {
        let form = match load(screen) {
            Ok(form) => form,
            Err(why) => {
                // A screen that will not load leaves the current one up. On a
                // panel this is where a log line would go; here it is stderr.
                eprintln!("{why}");
                return;
            }
        };
        let fit = form.fit(self.size);
        let root = self.ui.root();
        self.ui.remove(self.stage);
        self.stage = self
            .ui
            .add(root, Panel::filled(form.background()), fit.rect)
            .expect("the root takes a child");
        match form.build_fitted(&mut self.ui, self.stage, fit, &mut Wires) {
            Ok(built) => {
                self.built = built;
                self.screen = screen;
                self.dress();
            }
            Err(why) => eprintln!("{}: {why}", screen.file()),
        }
    }

    /// Puts the application's state onto the screen's widgets.
    fn dress(&mut self) {
        match self.screen {
            Screen::Hello => {
                let greeting = self.greeting.clone();
                if let Some(label) = self.label("greeting") {
                    label.set_text(greeting);
                }
            }
            Screen::Settings => {
                let (notify, volume) = (self.notify, self.volume);
                if let Some(id) = self.built.node("notify")
                    && let Some(check) = self.ui.widget_mut::<Checkbox<Message>>(id)
                {
                    check.set_checked(notify);
                }
                if let Some(id) = self.built.node("volume")
                    && let Some(slider) = self.ui.widget_mut::<Slider<Message>>(id)
                {
                    slider.set_value(volume);
                }
                self.summarise();
            }
        }
    }

    /// A named label on the current screen, if the file gave it that name.
    fn label(&mut self, name: &str) -> Option<&mut Label> {
        let id = self.built.node(name)?;
        self.ui.widget_mut::<Label>(id)
    }

    /// `hello`'s one piece of logic, unchanged — and the greeting is kept.
    fn greet(&mut self) {
        let name = self
            .built
            .node("who")
            .and_then(|id| self.ui.widget::<TextInput<Message>>(id))
            .map(|field| field.text().trim().to_string())
            .unwrap_or_default();
        self.greeting = if name.is_empty() {
            "Hello, whoever you are.".to_string()
        } else {
            format!("Hello, {name}.")
        };
        let greeting = self.greeting.clone();
        if let Some(label) = self.label("greeting") {
            label.set_text(greeting);
        }
    }

    /// Writes what the two settings currently are, on the settings screen.
    fn summarise(&mut self) {
        let text = format!(
            "release notes {}, volume {:.0}",
            if self.notify { "on" } else { "off" },
            self.volume
        );
        if let Some(label) = self.label("summary") {
            label.set_text(text);
        }
    }

    /// Hands every message the widgets sent to the function it names.
    ///
    /// The other half of [`Wires`]: that one turns a name into a message, this
    /// one turns the message into a call. Together they are the whole of what
    /// the files could not say.
    fn step(&mut self) {
        let messages: Vec<Message> = self.ui.drain_messages().collect();
        for message in messages {
            match message {
                Message::Greet => self.greet(),
                Message::Settings => self.show(Screen::Settings),
                Message::Back => self.show(Screen::Hello),
                Message::SetNotify(on) => {
                    self.notify = on;
                    self.summarise();
                }
                Message::SetVolume(value) => {
                    self.volume = value;
                    self.summarise();
                }
            }
        }
    }
}

impl DeniseApp for Runtime {
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
        self.step();

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

/// Draws one screen into a file, with no window and no event loop.
fn snapshot(path: &str, screen: Screen) -> std::io::Result<()> {
    use std::io::Write as _;

    let size = load(Screen::Hello).map_err(std::io::Error::other)?.size();
    let mut app = Runtime::new(size, 1.0);
    app.greet();
    if screen == Screen::Settings {
        app.notify = true;
        app.show(Screen::Settings);
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

#[cfg(test)]
mod tests {
    use super::*;
    use denise::{Modifiers, Point, PointerButton};

    const SIZE: Size = Size::new(460, 260);

    /// What a finger does to a named widget: down and up in its middle. The
    /// widget emits on the up, and `step` is what the loop does with it.
    fn press(app: &mut Runtime, name: &str) {
        let id = app
            .built
            .node(name)
            .unwrap_or_else(|| panic!("the screen names `{name}`"));
        let bounds = app.ui.bounds(id).expect("laid out");
        let at = Point::new(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
        let button = |state| InputEvent::PointerButton {
            button: PointerButton::Left,
            state,
            position: at,
            modifiers: Modifiers::NONE,
        };
        app.ui.handle(&[
            InputEvent::PointerMoved { position: at },
            button(ElementState::Down),
            button(ElementState::Up),
        ]);
        app.step();
    }

    fn label_text(app: &Runtime, name: &str) -> String {
        app.built
            .node(name)
            .and_then(|id| app.ui.widget::<Label>(id))
            .map(|label| label.text().to_string())
            .unwrap_or_default()
    }

    /// Both files load, which is the check that every name either uses is one
    /// `Wires` answers — made at test time, since it cannot be made at compile
    /// time on this path.
    #[test]
    fn both_screens_load_and_every_event_they_name_is_answered() {
        for screen in [Screen::Hello, Screen::Settings] {
            let form = load(screen).expect("the file is beside this crate and parses");
            assert_eq!(
                form.size(),
                SIZE,
                "{}: both screens share one size",
                screen.file()
            );
            let mut ui: Ui<Message> = Ui::new(SIZE, denise::theme::DARK);
            let root = ui.root();
            form.build(&mut ui, root, &mut Wires)
                .unwrap_or_else(|why| panic!("{}: {why}", screen.file()));
        }
    }

    /// The button on one screen brings in the other file, and its button brings
    /// the first back — a different set of named nodes each time.
    #[test]
    fn settings_swaps_the_screen_and_back_swaps_it_back() {
        let mut app = Runtime::new(SIZE, 1.0);
        assert_eq!(app.screen, Screen::Hello);
        assert!(app.built.node("who").is_some());
        assert!(app.built.node("notify").is_none(), "not on this screen");

        press(&mut app, "to-settings");
        assert_eq!(app.screen, Screen::Settings);
        assert!(app.built.node("notify").is_some());
        assert!(app.built.node("who").is_none(), "the hello screen is gone");

        press(&mut app, "back-button");
        assert_eq!(app.screen, Screen::Hello);
        assert!(app.built.node("who").is_some());
    }

    /// A checkbox's event arrives with its `bool`, changes the application's
    /// state, and the screen says so.
    #[test]
    fn an_event_carrying_a_value_reaches_the_state_with_it() {
        let mut app = Runtime::new(SIZE, 1.0);
        press(&mut app, "to-settings");
        assert!(!app.notify);
        assert_eq!(label_text(&app, "summary"), "release notes off, volume 35");

        press(&mut app, "notify");
        assert!(app.notify, "the checkbox's `true` arrived");
        assert_eq!(label_text(&app, "summary"), "release notes on, volume 35");

        press(&mut app, "notify");
        assert!(!app.notify, "and its `false`");

        // The slider's value goes the same way, and the state it lands in is
        // what the settings screen is dressed with next time.
        app.step();
        app.ui.drain_messages().for_each(drop);
        app.volume = 80.0;
        press(&mut app, "back-button");
        press(&mut app, "to-settings");
        let slider = app
            .built
            .node("volume")
            .and_then(|id| app.ui.widget::<Slider<Message>>(id))
            .expect("the slider");
        assert!(
            (slider.value() - 80.0).abs() < 0.5,
            "dressed with the state: {}",
            slider.value()
        );
        assert_eq!(label_text(&app, "summary"), "release notes off, volume 80");
    }

    /// The greeting is the application's, so it is still there after the
    /// screen it was typed on has been thrown away and built again.
    #[test]
    fn state_outlives_the_screen_it_was_set_on() {
        let mut app = Runtime::new(SIZE, 1.0);
        let who = app.built.node("who").expect("the field");
        app.ui
            .widget_mut::<TextInput<Message>>(who)
            .expect("a text input")
            .set_text(String::from("Ada"));
        press(&mut app, "greet-button");
        assert_eq!(label_text(&app, "greeting"), "Hello, Ada.");

        press(&mut app, "to-settings");
        press(&mut app, "back-button");
        assert_eq!(
            label_text(&app, "greeting"),
            "Hello, Ada.",
            "a fresh tree, dressed with the old greeting"
        );
    }

    /// A file naming an event nobody answers does not load, and the error
    /// names the event — the guarantee a redesign of a built application
    /// leans on.
    #[test]
    fn an_event_nobody_answers_fails_at_load_naming_it() {
        let text = std::fs::read_to_string(forms_dir().join("hello.dform"))
            .unwrap()
            .replace("on-press=settings", "on-press=nope");
        let form = Form::parse_within(&text, PATIENCE).expect("still a form");
        let mut ui: Ui<Message> = Ui::new(SIZE, denise::theme::DARK);
        let root = ui.root();
        let why = form
            .build(&mut ui, root, &mut Wires)
            .expect_err("`nope` is nobody's")
            .to_string();
        assert!(
            why.contains("nope"),
            "the error does not name the event: {why}"
        );
    }
}
