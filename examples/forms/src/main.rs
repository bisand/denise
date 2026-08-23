//! Secondary windows: a modeless settings form, a modal, and the state they share.
//!
//! ```text
//! cargo run -p forms
//! cargo run -p forms -- --settings     # with the settings form already open
//! cargo run -p forms -- --delete       # with the modal already up
//! ```
//!
//! Every other example in this repository puts its dialogs *inside* one surface,
//! because that is what a kiosk and an embedded control can do. This one is the
//! desktop exception: a window manager exists, so a form can have a window.
//!
//! # What is here
//!
//! - **A modeless form.** `Settings…` opens a second window that belongs to this
//!   one — above it, closed with it — and leaves the main window fully usable.
//!   Edits show up here as they are made.
//! - **A modal.** `Delete…` opens a window the main window cannot be used behind.
//!   The same question `table-editor` asks with `Ui::push_scene`, in a window.
//! - **A modal over the modeless form.** `Reset…` inside the settings form is
//!   owned by *that* window: it blocks the form and not the main window, and
//!   closing the form takes it along. Ownership follows whoever asked.
//! - **State they share.** An `Rc<RefCell<Shared>>` the application creates and
//!   hands out. Nothing in the backend knows it exists.
//!
//! # What the toolkit does and does not do
//!
//! `denise-winit` supplies a window, a surface and a place in its event loop. That
//! is all. A form is an ordinary [`DeniseApp`] — same trait as this file's main
//! window, same `Ui`, same message loop — so there is no form type to learn, no
//! base class, and nothing that makes a "dialog" different from a "window" except
//! the [`Modality`] the application asked for.
//!
//! **This is a desktop-only capability and the crate that has it is the desktop
//! backend.** A kiosk build links `denise-drm` or `denise-fbdev`, never sees any
//! of this, and asks its questions with `Ui::push_scene` — which is why this
//! example, alone among them, has no `kiosk` feature.

mod confirm;
mod settings;
mod shared;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use denise::{DamageTracker, ElementState, Frame, InputEvent, KeyCode, Rect, Role, Size, theme};
use denise_ui::widgets::{Button, Label, Panel};
use denise_ui::{NodeId, Ui};
use denise_winit::{DeniseApp, WindowConfig, WindowRequest, run_with};

use shared::Shared;

const WINDOW: Size = Size::new(540, 320);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Message {
    Settings,
    Delete,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Opening a form on the first frame, so the thing this example is about does
    // not need a hand on the mouse to look at. `table-editor --keyboard` is the
    // same idea.
    let mut opening = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--settings" => opening = Some(Message::Settings),
            "--delete" => opening = Some(Message::Delete),
            other => eprintln!("ignoring {other}"),
        }
    }

    run_with(
        WindowConfig {
            title: "Denise — forms".into(),
            size: WINDOW,
            resizable: false,
            ..WindowConfig::default()
        },
        move |size, scale| {
            let mut app = Main::new(size, scale);
            if let Some(message) = opening {
                app.handle(message);
            }
            app
        },
    )?;
    Ok(())
}

struct Main {
    ui: Ui<Message>,
    /// The one copy every window is given a handle to.
    shared: Rc<RefCell<Shared>>,
    heading: NodeId,
    subtitle: NodeId,
    reading: NodeId,
    record: NodeId,
    settings_button: NodeId,
    /// Windows asked for this frame, taken by the backend at the end of it.
    pending: Vec<WindowRequest>,
    seen: u64,
    exit: bool,
    started: Instant,
}

impl Main {
    fn new(size: Size, scale: f32) -> Self {
        let s = |r: Rect| r.scaled(scale);
        let px = |v: f32| (v * scale + 0.5) as u16;

        let shared = Rc::new(RefCell::new(Shared::default()));
        let mut ui: Ui<Message> = Ui::new(size, theme::DARK.scaled(scale));
        ui.show_cursor(false);
        let root = ui.root();

        let card = ui
            .add(root, Panel::default(), s(Rect::new(20, 20, 500, 280)))
            .expect("card");

        let heading = ui
            .add(
                card,
                Label::new(shared.borrow().title.clone()).with_size(px(24.0)),
                s(Rect::new(28, 28, 444, 32)),
            )
            .expect("heading");
        let subtitle = ui
            .add(
                card,
                Label::new("Countess of Lovelace").with_size(px(15.0)),
                s(Rect::new(28, 66, 444, 22)),
            )
            .expect("subtitle");
        let reading = ui
            .add(
                card,
                Label::new("").with_size(px(15.0)),
                s(Rect::new(28, 104, 444, 22)),
            )
            .expect("reading");
        let record = ui
            .add(
                card,
                Label::new("").with_size(px(15.0)),
                s(Rect::new(28, 132, 444, 22)),
            )
            .expect("record");

        let settings_button = ui
            .add(
                card,
                Button::new("Settings...", Message::Settings)
                    .with_role(Role::Primary)
                    .with_size(px(15.0)),
                s(Rect::new(28, 200, 150, 36)),
            )
            .expect("settings button");
        ui.add(
            card,
            Button::new("Delete...", Message::Delete)
                .with_role(Role::Error)
                .with_size(px(15.0)),
            s(Rect::new(198, 200, 150, 36)),
        );

        let mut this = Self {
            ui,
            shared,
            heading,
            subtitle,
            reading,
            record,
            settings_button,
            pending: Vec::new(),
            seen: u64::MAX,
            exit: false,
            started: Instant::now(),
        };
        this.refresh();
        this
    }

    /// Writes the shared state into the widgets that display it.
    ///
    /// `widget_mut` marks the node it hands back for repaint, so a title that has
    /// not changed is deliberately not written: writing it anyway would repaint
    /// the heading twenty times a second for nothing.
    fn refresh(&mut self) {
        // Copied out and the borrow dropped before anything below touches the
        // tree: a form running in another window holds the same `RefCell`, and a
        // borrow left open across a repaint is a panic waiting for a coincidence.
        let shared = self.shared.borrow();
        let title = shared.title.clone();
        let subtitle = shared.subtitle;
        let brightness = shared.brightness;
        let record = shared.record;
        let settings_open = shared.settings_open;
        drop(shared);

        set_label(&mut self.ui, self.heading, title);
        set_label(
            &mut self.ui,
            self.subtitle,
            if subtitle { "Countess of Lovelace" } else { "" }.to_string(),
        );
        set_label(
            &mut self.ui,
            self.reading,
            format!("Brightness {brightness:.0}%"),
        );
        set_label(
            &mut self.ui,
            self.record,
            if record {
                "The record is here.".into()
            } else {
                "The record was deleted.".to_string()
            },
        );
        // The button says what the second press will do, which is the reason the
        // application tracks whether the form is open — not the backend's rule,
        // and not something it could keep for us.
        set_button(
            &mut self.ui,
            self.settings_button,
            if settings_open {
                "Close settings"
            } else {
                "Settings..."
            },
        );
    }

    fn handle(&mut self, message: Message) {
        match message {
            Message::Settings => {
                let open = self.shared.borrow().settings_open;
                if open {
                    // No handle to a window somebody else owns, and none wanted:
                    // the form is asked, and closes itself.
                    self.shared.borrow_mut().settings_should_close = true;
                } else {
                    self.shared.borrow_mut().settings_open = true;
                    self.pending.push(settings::open(self.shared.clone()));
                }
            }

            Message::Delete => self.pending.push(confirm::ask(
                "Delete record",
                "Delete this record?",
                "Delete",
                self.shared.clone(),
                |shared| shared.record = false,
            )),
        }
    }
}

fn set_label(ui: &mut Ui<Message>, id: NodeId, text: String) {
    if let Some(label) = ui.widget::<Label>(id)
        && label.text() == text
    {
        return;
    }
    if let Some(label) = ui.widget_mut::<Label>(id) {
        label.set_text(text);
    }
}

fn set_button(ui: &mut Ui<Message>, id: NodeId, text: &str) {
    if let Some(button) = ui.widget::<Button<Message>>(id)
        && button.label() == text
    {
        return;
    }
    if let Some(button) = ui.widget_mut::<Button<Message>>(id) {
        button.set_label(text);
    }
}

impl DeniseApp for Main {
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
            self.handle(message);
        }

        // Anything a form changed since the last look, including a form closing.
        let revision = self.shared.borrow().revision;
        if revision != self.seen {
            self.refresh();
            self.seen = revision;
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

    /// Everything the application asked for this frame.
    ///
    /// The backend takes this list, opens a window for each request and owns none
    /// of what goes in them.
    fn take_windows(&mut self) -> Vec<WindowRequest> {
        std::mem::take(&mut self.pending)
    }

    fn exit_requested(&self) -> bool {
        self.exit
    }

    /// Animation, or the watch interval, whichever comes first.
    ///
    /// This window displays state its forms edit, so it cannot sleep until
    /// somebody clicks it. See [`shared::WATCH`].
    fn next_frame_in(&self) -> Option<Duration> {
        let now = self.started.elapsed().as_millis() as u64;
        let animating = self
            .ui
            .next_wake_ms()
            .map(|wake| Duration::from_millis(wake.saturating_sub(now)));
        Some(animating.map_or(shared::WATCH, |wake| wake.min(shared::WATCH)))
    }
}
