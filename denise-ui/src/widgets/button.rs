//! A pressable, focusable, message-emitting rectangle.

use alloc::string::String;

use denise::{ElementState, InputEvent, KeyCode, Radius, Role};
use denise_render::Canvas;
use denise_text::TextStyle;

use crate::widget::{Animation, Event, EventCtx, Handled, PaintCtx, VisualState, Widget};
use crate::widgets::style::{Align, draw_aligned, focus_ring, interactive_pair};

/// A button that emits a message when it is activated.
///
/// Activation is a release *inside* the button, or Enter/Space while it holds
/// focus. A press that is dragged off and released elsewhere is cancelled, which
/// is what makes a touchscreen usable — a finger that lands on the wrong control
/// can be slid away rather than committing.
#[derive(Clone, Debug)]
pub struct Button<M> {
    label: String,
    message: Option<M>,
    role: Role,
    radius: Radius,
    style: TextStyle,
    no_focus: bool,
    /// How long a hold waits, and how fast it goes after that. `None` is a
    /// button that does not repeat, which is nearly all of them.
    repeat: Option<Repeat>,
    /// When the finger went down, while it is still down.
    held_since: Option<u64>,
    /// The last repeat already counted, as a count rather than a time so that
    /// arithmetic on a clock that jumped cannot produce a burst.
    counted: u32,
    /// Repeats owed to whoever asks next.
    pending: u32,
}

/// A button's press-and-hold schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Repeat {
    delay_ms: u64,
    interval_ms: u64,
}

/// The most repeats one frame may hand over.
///
/// A loop that blocked — a page arriving over a slow link, a display waking,
/// a snapshot ticking straight past a second — comes back with a clock that has
/// jumped, and counting from the press would then report every repeat that gap
/// covered. Truthfully, and uselessly: nobody watching a frozen screen meant to
/// delete two hundred characters, and a keyboard that empties a field because
/// the network hiccuped is worse than one that does not repeat at all.
///
/// Past this the missed repeats are dropped rather than queued, so a stall
/// costs the repeats it swallowed and nothing more. Four is about a quarter of
/// a second at the on-screen keyboard's interval: enough that an ordinary
/// stutter is invisible, little enough that a real stall is obvious.
const MAX_CATCH_UP: u32 = 4;

impl<M> Button<M> {
    /// A primary button carrying `message`.
    pub fn new(label: impl Into<String>, message: M) -> Self {
        Self {
            label: label.into(),
            message: Some(message),
            role: Role::Primary,
            radius: Radius::Field,
            style: TextStyle::built_in(16),
            no_focus: false,
            repeat: None,
            held_since: None,
            counted: 0,
            pending: 0,
        }
    }

    /// A button that emits nothing. Useful as a disabled affordance, or when the
    /// application only cares about focus.
    pub fn inert(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            message: None,
            role: Role::Primary,
            radius: Radius::Field,
            style: TextStyle::built_in(16),
            no_focus: false,
            repeat: None,
            held_since: None,
            counted: 0,
            pending: 0,
        }
    }

    /// Presses without touching focus: the button takes none, and costs none.
    ///
    /// An ordinary button takes focus when pressed, which is right for a button
    /// somebody tabs to and wrong for a key on an on-screen keyboard — that key
    /// is pressed *while* a field is being typed into, and the field has to keep
    /// the caret. Making the key merely unfocusable is not enough either, since
    /// pressing an unfocusable node is what drops focus and commits a field.
    ///
    /// So this asks for neither. The button still presses, still paints pressed,
    /// still emits its message; Tab skips it, and the focus ring never moves.
    ///
    /// ```
    /// # use denise::{Rect, Size, theme};
    /// # use denise_ui::{Ui, widgets::{Button, TextInput}};
    /// # #[derive(Clone, Debug)] enum Msg { Key(char) }
    /// # let mut ui: Ui<Msg> = Ui::new(Size::new(800, 480), theme::DARK);
    /// # let root = ui.root();
    /// # let field = ui.add(root, TextInput::new(), Rect::new(0, 0, 200, 40)).unwrap();
    /// # ui.focus(Some(field));
    /// ui.add(root, Button::new("q", Msg::Key('q')).no_focus(), Rect::new(0, 100, 40, 40));
    /// assert_eq!(ui.focused(), Some(field));
    /// ```
    pub fn no_focus(mut self) -> Self {
        self.no_focus = true;
        self
    }

    /// Emits again, and again, while a finger stays on it.
    ///
    /// A repeating button **acts on press rather than on release**, which is the
    /// only way it could work: repeats have to start while the finger is still
    /// down. That is a real change in feel and the reason this is opt-in — an
    /// ordinary button emits on release precisely so that sliding off it before
    /// letting go cancels the press, and a repeating one gives that up.
    ///
    /// Reach for it where holding means *more of the same*: Backspace on an
    /// on-screen keyboard, a stepper's arrows, a scrollbar's ends. Not for
    /// anything whose second press means something different from its first.
    ///
    /// `delay_ms` is the pause before the first repeat — long enough that an
    /// ordinary tap never triggers one — and `interval_ms` the gap between the
    /// rest. The repeats are counted, not emitted: the button has no message
    /// channel while it is animating, so whoever owns it collects them with
    /// [`take_repeats`](Self::take_repeats) once a frame.
    ///
    /// Costs nothing when nothing is held. The button asks the tree to wake it
    /// only between a press and its release, and answers
    /// [`Wake::Never`](crate::Wake::Never) the moment the finger goes.
    pub fn with_repeat(mut self, delay_ms: u64, interval_ms: u64) -> Self {
        self.repeat = Some(Repeat {
            delay_ms,
            interval_ms: interval_ms.max(1),
        });
        self
    }

    /// Repeats owed since this was last called, and clears the tally.
    ///
    /// Zero unless [`with_repeat`](Self::with_repeat) was asked for and a finger
    /// has been resting on the button for longer than its delay.
    pub fn take_repeats(&mut self) -> u32 {
        core::mem::take(&mut self.pending)
    }

    /// Whether a finger is on it now.
    #[inline]
    pub const fn is_held(&self) -> bool {
        self.held_since.is_some()
    }

    /// Sets the colour role. The content colour comes from the theme's pairing, so
    /// the label stays readable whichever role and theme are chosen.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Sets the corner rounding token.
    pub fn with_radius(mut self, radius: Radius) -> Self {
        self.radius = radius;
        self
    }

    /// Sets the font and size.
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets the size, keeping the font.
    pub fn with_size(mut self, size_px: u16) -> Self {
        self.style.size_px = size_px;
        self
    }

    /// The font and size the label draws in.
    #[inline]
    pub const fn style(&self) -> TextStyle {
        self.style
    }

    /// The current label.
    #[inline]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Replaces the label.
    pub fn set_label(&mut self, label: impl Into<String>) {
        self.label = label.into();
    }

    /// Replaces the message emitted on activation.
    pub fn set_message(&mut self, message: Option<M>) {
        self.message = message;
    }

    /// Replaces the colour role.
    ///
    /// What a list of buttons uses to show which one is selected, since a role
    /// survives a theme change and a colour does not.
    pub fn set_role(&mut self, role: Role) {
        self.role = role;
    }

    /// Replaces the font and size.
    ///
    /// For an application that registers a font after building its tree, which is
    /// the ordinary case: the tree has to exist before anyone knows whether the
    /// font file was there.
    pub fn set_style(&mut self, style: TextStyle) {
        self.style = style;
    }

    /// Width this button needs for its label plus comfortable padding.
    ///
    /// Takes the engine because with a proportional font the answer is not the
    /// character count times anything, and guessing is how a button ends up one
    /// letter too narrow in the language it was not tested in.
    pub fn preferred_width(&self, engine: &mut denise_text::TextEngine) -> i32 {
        let text = engine.measure_line(self.style, &self.label);
        text + i32::from(self.style.size_px) * 3 / 2
    }
}

impl<M: Clone + 'static> Widget<M> for Button<M> {
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        let radius = ctx.theme.radius(self.radius);
        let (background, content) = interactive_pair(ctx.theme, self.role, ctx.state);
        canvas.fill_rounded_rect(ctx.bounds, radius, background);
        if ctx.state.contains(VisualState::FOCUSED) {
            focus_ring(ctx.theme, ctx.bounds, radius, canvas);
        }
        draw_aligned(
            canvas,
            ctx.text,
            self.style,
            ctx.bounds,
            (Align::Center, Align::Center),
            &self.label,
            content,
        );
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        // A repeating button lives on the down edge instead of the up one, and
        // has to, since the repeats begin while the finger is still there.
        if self.repeat.is_some() {
            match event {
                Event::Input(
                    InputEvent::PointerButton {
                        state: ElementState::Down,
                        position,
                        ..
                    }
                    | InputEvent::TouchDown { position, .. },
                ) if ctx.bounds.contains(*position) => {
                    self.held_since = Some(ctx.now_ms);
                    self.counted = 0;
                    // The wake that carries the hold. Asked for here and given
                    // back by `animate` the moment the finger leaves, so a tree
                    // nobody is touching schedules nothing.
                    ctx.request_animation();
                    if let Some(message) = self.message.clone() {
                        ctx.emit(message);
                    }
                    return Handled::Yes;
                }
                // Every way a press can end. The tree dispatches the release to
                // whichever node was pressed even when the finger has wandered
                // off it, and `PressCancelled` covers the ends that no pointer
                // event describes.
                Event::Input(InputEvent::PointerButton {
                    state: ElementState::Up,
                    ..
                })
                | Event::Input(InputEvent::TouchUp { .. })
                | Event::PressCancelled => {
                    self.held_since = None;
                    // Whatever was owed is dropped with the press: a repeat
                    // nobody collected before the finger lifted is one the user
                    // did not wait for.
                    self.pending = 0;
                    return Handled::Yes;
                }
                _ => {}
            }
        }

        let activated = match event {
            Event::Input(InputEvent::PointerButton {
                state: ElementState::Up,
                position,
                ..
            }) => ctx.bounds.contains(*position),
            Event::Input(InputEvent::TouchUp {
                position,
                cancelled: false,
                ..
            }) => ctx.bounds.contains(*position),
            Event::Input(InputEvent::Key {
                code: KeyCode::Enter | KeyCode::Space | KeyCode::NumpadEnter,
                state: ElementState::Down,
                repeat: false,
                ..
            }) => ctx.state.contains(VisualState::FOCUSED),
            _ => return Handled::No,
        };
        if !activated {
            return Handled::No;
        }
        if let Some(message) = self.message.clone() {
            ctx.emit(message);
        }
        Handled::Yes
    }

    /// Counts the repeats a held finger has earned, and asks for the next wake.
    ///
    /// Counted from the press rather than accumulated from the last tick, so a
    /// clock that jumped — a loop that blocked, a snapshot ticking straight past
    /// a second — yields the repeats that time actually covered and no more.
    fn animate(&mut self, now_ms: u64) -> Animation {
        let (Some(repeat), Some(since)) = (self.repeat, self.held_since) else {
            return Animation::NONE;
        };
        let held = now_ms.saturating_sub(since);
        let Some(after_delay) = held.checked_sub(repeat.delay_ms) else {
            // Still inside the initial pause: nothing owed, come back when it
            // is over.
            return Animation::due_at(since + repeat.delay_ms);
        };
        // The first repeat lands *at* the delay, so a hold of exactly the delay
        // owes one.
        let due = u32::try_from(after_delay / repeat.interval_ms + 1).unwrap_or(u32::MAX);
        let owed = due.saturating_sub(self.counted).min(MAX_CATCH_UP);
        self.pending = self.pending.saturating_add(owed);
        // Counted up to `due` rather than to what was handed over, so the
        // repeats a stall swallowed are gone rather than owed.
        self.counted = due;
        Animation::due_at(
            since
                .saturating_add(repeat.delay_ms)
                .saturating_add(u64::from(due).saturating_mul(repeat.interval_ms)),
        )
    }

    /// A held key under reduced motion still repeats.
    ///
    /// `Motion::None` is about movement, not about clocks — the same rule the
    /// carousel's auto-advance follows. Backspace that stopped deleting because
    /// somebody turned animations off would be a bug wearing a setting's
    /// clothes.
    fn snap(&mut self, now_ms: u64) -> Animation {
        self.animate(now_ms)
    }

    fn accepts_pointer(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        !self.no_focus
    }

    fn preserves_focus(&self) -> bool {
        self.no_focus
    }
}
