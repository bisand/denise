//! A modal, in a window of its own.
//!
//! The same question `table-editor` asks with `Ui::push_scene` and a dimmed
//! backdrop, moved out into its own window — which is the whole point of the
//! comparison. Everything about it that matters is unchanged: it takes the input,
//! its owner stops listening until it is answered, and the answer arrives in the
//! application rather than in a callback somewhere.
//!
//! It is also the only form here that is genuinely reusable: the caller supplies
//! the question and what to do about a yes.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use denise::{DamageTracker, ElementState, Frame, InputEvent, KeyCode, Rect, Role, Size, theme};
use denise_ui::Ui;
use denise_ui::widgets::{Button, Label, Panel};
use denise_winit::{DeniseApp, Modality, WindowConfig, WindowRequest};

use crate::shared::Shared;

/// Logical size of the dialog. Small, fixed, and not resizable — a question with
/// two answers has no layout to reflow.
pub const SIZE: Size = Size::new(380, 170);

/// What a yes does. Supplied by whoever asked the question, which is why this
/// dialog is the one form here that both windows can use.
type Accept = Box<dyn FnOnce(&mut Shared)>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Answer {
    Yes,
    No,
}

/// Asks `question`, and runs `accept` on a yes.
///
/// Returns the request rather than a window: nothing here can open one, and that
/// is the shape of the API — the application hands a request back from
/// [`DeniseApp::take_windows`] and the runner does the rest.
pub fn ask(
    title: &str,
    question: &'static str,
    confirm: &'static str,
    shared: Rc<RefCell<Shared>>,
    accept: impl FnOnce(&mut Shared) + 'static,
) -> WindowRequest {
    let config = WindowConfig {
        title: title.into(),
        size: SIZE,
        resizable: false,
        ..WindowConfig::default()
    };
    WindowRequest::new(config, move |size, scale| {
        Confirm::new(size, scale, question, confirm, shared, accept)
    })
    .with_modality(Modality::Modal)
}

struct Confirm {
    ui: Ui<Answer>,
    shared: Rc<RefCell<Shared>>,
    /// Runs on a yes. `Option`, because it runs at most once.
    accept: Option<Accept>,
    /// Set when the question has been answered; read by `exit_requested`.
    done: bool,
    started: Instant,
}

impl Confirm {
    fn new(
        size: Size,
        scale: f32,
        question: &str,
        confirm: &str,
        shared: Rc<RefCell<Shared>>,
        accept: impl FnOnce(&mut Shared) + 'static,
    ) -> Self {
        let s = |r: Rect| r.scaled(scale);
        let px = |v: f32| (v * scale + 0.5) as u16;

        let mut ui: Ui<Answer> = Ui::new(size, theme::DARK.scaled(scale));
        // The window system draws a pointer already, so the tree must not draw a
        // second one over it.
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
            Label::new(question).with_size(px(17.0)),
            s(Rect::new(24, 32, 332, 24)),
        );

        let no = ui.add(
            card,
            Button::new("Cancel", Answer::No).with_size(px(15.0)),
            s(Rect::new(24, 108, 120, 34)),
        );
        ui.add(
            card,
            Button::new(confirm, Answer::Yes)
                .with_role(Role::Error)
                .with_size(px(15.0)),
            s(Rect::new(236, 108, 120, 34)),
        );

        // Focus lands on the safe answer, so Enter on a dialog nobody read does
        // the harmless thing. The same rule as the in-surface modal, for the same
        // reason.
        ui.focus(no);

        Self {
            ui,
            shared,
            accept: Some(Box::new(accept)),
            done: false,
            started: Instant::now(),
        }
    }

    fn answer(&mut self, answer: Answer) {
        if answer == Answer::Yes
            && let Some(accept) = self.accept.take()
        {
            let mut shared = self.shared.borrow_mut();
            shared.change(accept);
        }
        self.done = true;
    }
}

impl DeniseApp for Confirm {
    fn update(&mut self, events: &[InputEvent], damage: &mut DamageTracker) {
        for event in events {
            if let InputEvent::Key {
                code: KeyCode::Escape,
                state: ElementState::Down,
                ..
            } = event
            {
                self.answer(Answer::No);
            }
        }

        self.ui.handle(events);
        self.ui.tick(self.started.elapsed().as_millis() as u64);

        let answers: Vec<Answer> = self.ui.drain_messages().collect();
        for answer in answers {
            self.answer(answer);
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
        self.done
    }

    fn next_frame_in(&self) -> Option<Duration> {
        let now = self.started.elapsed().as_millis() as u64;
        self.ui
            .next_wake_ms()
            .map(|wake| Duration::from_millis(wake.saturating_sub(now)))
    }
}
