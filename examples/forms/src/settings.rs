//! A modeless form, in a window of its own.
//!
//! The interesting one. It is not a dialog: the main window keeps working while
//! this is open, both windows draw at the same time, and every edit here shows up
//! there immediately. It is also an ordinary [`DeniseApp`] — the same trait the
//! main window implements, built the same way, with its own tree and its own
//! message type. There is no "form" type in the toolkit and no base class here;
//! a form is a small program that happens to be handed a window.
//!
//! Two things in this file are worth reading for the pattern rather than the
//! example:
//!
//! - **`Drop` clears the open flag.** A window can close five ways — the button
//!   here, its title bar, its owner closing, the main window asking, the run
//!   ending — and all five end with this application being dropped. Bookkeeping
//!   that lives anywhere else gets one of them wrong.
//! - **It opens its own modal.** `Reset…` is owned by *this* window, not by the
//!   main one: it blocks this form, it sits above this form, and closing this form
//!   takes it along. Ownership follows whoever asked.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use denise::{DamageTracker, ElementState, Frame, InputEvent, KeyCode, Rect, Role, Size, theme};
use denise_ui::widgets::{Button, Checkbox, Label, Panel, Slider, TextInput};
use denise_ui::{NodeId, Ui};
use denise_winit::{DeniseApp, WindowConfig, WindowRequest};

use crate::confirm;
use crate::shared::Shared;

/// Logical size of the form.
pub const SIZE: Size = Size::new(400, 330);

#[derive(Clone, Copy, PartialEq)]
enum Message {
    /// Read the title field and apply it. Enter in the field sends this too.
    Apply,
    Subtitle(bool),
    Brightness(f32),
    Reset,
    Close,
}

/// The request that opens this form. [`Modality::Owned`] by default, which is
/// what modeless-but-belonging-to-this-application means.
///
/// [`Modality::Owned`]: denise_winit::Modality::Owned
pub fn open(shared: Rc<RefCell<Shared>>) -> WindowRequest {
    let config = WindowConfig {
        title: "Settings".into(),
        size: SIZE,
        resizable: false,
        ..WindowConfig::default()
    };
    WindowRequest::new(config, move |size, scale| {
        Settings::new(size, scale, shared)
    })
}

struct Settings {
    ui: Ui<Message>,
    shared: Rc<RefCell<Shared>>,
    title: NodeId,
    reading: NodeId,
    /// Modals this form has asked for, taken by the backend next frame.
    pending: Vec<WindowRequest>,
    /// Set by the Close button; the title bar goes through `close_requested`.
    closing: bool,
    /// The last revision this form has caught up with. See `update`.
    seen: u64,
    started: Instant,
}

impl Settings {
    fn new(size: Size, scale: f32, shared: Rc<RefCell<Shared>>) -> Self {
        let s = |r: Rect| r.scaled(scale);
        let px = |v: f32| (v * scale + 0.5) as u16;

        let (title_text, subtitle, brightness) = {
            let shared = shared.borrow();
            (shared.title.clone(), shared.subtitle, shared.brightness)
        };

        let mut ui: Ui<Message> = Ui::new(size, theme::DARK.scaled(scale));
        ui.show_cursor(false);
        let root = ui.root();
        let card = ui
            .add(
                root,
                Panel::default(),
                s(Rect::new(0, 0, SIZE.width as i32, SIZE.height as i32)),
            )
            .expect("card");

        ui.add(
            card,
            Label::new("Settings").with_size(px(20.0)),
            s(Rect::new(24, 20, 352, 26)),
        );

        ui.add(
            card,
            Label::new("Title").with_size(px(14.0)),
            s(Rect::new(24, 62, 352, 18)),
        );
        let mut field = TextInput::<Message>::new()
            .with_placeholder("a name")
            .with_submit(Message::Apply)
            .with_size(px(15.0));
        field.set_text(title_text);
        let title = ui
            .add(card, field, s(Rect::new(24, 82, 250, 32)))
            .expect("title field");
        ui.add(
            card,
            Button::new("Apply", Message::Apply).with_size(px(14.0)),
            s(Rect::new(284, 82, 92, 32)),
        );

        ui.add(
            card,
            Checkbox::new("Show the subtitle", Message::Subtitle)
                .with_checked(subtitle)
                .with_size(px(15.0)),
            s(Rect::new(24, 130, 352, 24)),
        );

        ui.add(
            card,
            Label::new("Brightness").with_size(px(14.0)),
            s(Rect::new(24, 168, 200, 18)),
        );
        let reading = ui
            .add(
                card,
                Label::new(format!("{brightness:.0}%")).with_size(px(14.0)),
                s(Rect::new(316, 168, 60, 18)),
            )
            .expect("reading");
        ui.add(
            card,
            Slider::new(0.0, 100.0, brightness, Message::Brightness).with_step(1.0),
            s(Rect::new(24, 192, 352, 28)),
        );

        ui.add(
            card,
            Button::new("Reset...", Message::Reset).with_size(px(14.0)),
            s(Rect::new(24, 264, 110, 34)),
        );
        ui.add(
            card,
            Button::new("Close", Message::Close)
                .with_role(Role::Primary)
                .with_size(px(14.0)),
            s(Rect::new(266, 264, 110, 34)),
        );

        ui.focus(Some(title));

        Self {
            ui,
            shared,
            title,
            reading,
            pending: Vec::new(),
            closing: false,
            seen: 0,
            started: Instant::now(),
        }
    }

    fn handle(&mut self, message: Message) {
        match message {
            Message::Apply => {
                let text = self
                    .ui
                    .widget::<TextInput<Message>>(self.title)
                    .map(|field| field.text().trim().to_string())
                    .unwrap_or_default();
                if !text.is_empty() {
                    self.shared
                        .borrow_mut()
                        .change(|shared| shared.title = text);
                }
            }

            Message::Subtitle(on) => {
                self.shared
                    .borrow_mut()
                    .change(|shared| shared.subtitle = on);
            }

            Message::Brightness(value) => {
                self.shared
                    .borrow_mut()
                    .change(|shared| shared.brightness = value);
                if let Some(label) = self.ui.widget_mut::<Label>(self.reading) {
                    label.set_text(format!("{value:.0}%"));
                }
            }

            // Owned by this window, because this window asked. It blocks this form
            // and leaves the main window alone.
            Message::Reset => self.pending.push(confirm::ask(
                "Reset settings",
                "Put every setting back to its default?",
                "Reset",
                self.shared.clone(),
                |shared| {
                    let defaults = Shared::default();
                    shared.title = defaults.title;
                    shared.subtitle = defaults.subtitle;
                    shared.brightness = defaults.brightness;
                },
            )),

            Message::Close => self.closing = true,
        }
    }

    /// Pulls the widgets back into line with state something else changed.
    ///
    /// Only the modal does that here — a reset from a window this form opened —
    /// so this runs about once a session. It is still worth having: a form that
    /// displays shared state and does not re-read it is a form showing the last
    /// thing *it* did.
    fn refresh(&mut self) {
        let (title, brightness) = {
            let shared = self.shared.borrow();
            (shared.title.clone(), shared.brightness)
        };
        if let Some(field) = self.ui.widget_mut::<TextInput<Message>>(self.title)
            && field.text() != title
        {
            field.set_text(title);
        }
        if let Some(label) = self.ui.widget_mut::<Label>(self.reading) {
            label.set_text(format!("{brightness:.0}%"));
        }
    }
}

/// Every way this window can close ends here, which is why the bookkeeping is
/// here and not in any of them.
impl Drop for Settings {
    fn drop(&mut self) {
        let mut shared = self.shared.borrow_mut();
        shared.settings_open = false;
        shared.settings_should_close = false;
        shared.revision += 1;
    }
}

impl DeniseApp for Settings {
    fn update(&mut self, events: &[InputEvent], damage: &mut DamageTracker) {
        for event in events {
            if let InputEvent::Key {
                code: KeyCode::Escape,
                state: ElementState::Down,
                ..
            } = event
            {
                self.closing = true;
            }
        }

        // Caught up with before anything else, so an edit made below is not
        // immediately overwritten by the value it replaced.
        let revision = self.shared.borrow().revision;
        if revision != self.seen {
            self.refresh();
        }

        self.ui.handle(events);
        self.ui.tick(self.started.elapsed().as_millis() as u64);

        let messages: Vec<Message> = self.ui.drain_messages().collect();
        for message in messages {
            self.handle(message);
        }

        // Whatever the messages above changed is this form's own doing and needs
        // no refresh; anything that arrives later is somebody else's.
        self.seen = self.shared.borrow().revision;

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

    fn take_windows(&mut self) -> Vec<WindowRequest> {
        std::mem::take(&mut self.pending)
    }

    fn exit_requested(&self) -> bool {
        self.closing || self.shared.borrow().settings_should_close
    }

    /// Animation, or the watch interval, whichever comes first.
    ///
    /// This form displays state a window it opened can change, so it cannot sleep
    /// until somebody types in it. See [`shared::WATCH`](crate::shared::WATCH).
    fn next_frame_in(&self) -> Option<Duration> {
        let now = self.started.elapsed().as_millis() as u64;
        let animating = self
            .ui
            .next_wake_ms()
            .map(|wake| Duration::from_millis(wake.saturating_sub(now)));
        Some(animating.map_or(crate::shared::WATCH, |wake| wake.min(crate::shared::WATCH)))
    }
}
