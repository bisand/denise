//! A pressable, focusable, message-emitting rectangle.

use alloc::string::String;

use denise::{ElementState, InputEvent, KeyCode, Radius, Rect, Role};
use denise_render::Canvas;
use denise_render::icon::Icon;
use denise_text::TextStyle;

use crate::widget::{Animation, Event, EventCtx, Handled, PaintCtx, VisualState, Widget};
use crate::widgets::describe::{
    Describe, DynDescribe, Mismatch, Payload, Property, PropertyKind, RADII, ROLES, Value,
};
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
    /// A small second label in the top-right corner. Empty for most buttons.
    corner: String,
    /// A drawn shape in place of the label. `None` for most buttons.
    icon: Option<&'static Icon>,
    /// Whether this button reports how long it has been held.
    watches_hold: bool,
    /// How long the current press has lasted, as of the last `animate`.
    held_ms: u64,
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

/// How much of a button's shorter side an icon takes, as a percentage.
///
/// Slightly over half. A glyph in a 48-pixel key occupies about this much once
/// its own side bearings are counted, so an icon at the same share sits in a row
/// of lettered keys without looking like a different size of thing.
const ICON_SHARE: i32 = 55;

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
            corner: String::new(),
            icon: None,
            watches_hold: false,
            held_ms: 0,
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
            corner: String::new(),
            icon: None,
            watches_hold: false,
            held_ms: 0,
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

    /// A shape drawn in place of the label.
    ///
    /// For the button whose meaning is a picture — a Backspace key, a
    /// scrollbar's arrow — and specifically for the case where that picture is
    /// not reliably in the font. An [`Icon`] is filled polygons this crate
    /// draws itself, so it is the same on a machine with no fonts installed at
    /// all as it is on one with DejaVu.
    ///
    /// It takes the label's place rather than sitting beside it, and it is
    /// drawn in the same content colour the label would have used, so it
    /// follows the theme and the button's state without being told. Keep the
    /// label anyway: it is what [`label`](Self::label) still reports, which is
    /// what a test and an accessibility pass read.
    ///
    /// Sized from the button rather than fixed, and kept square — the shortest
    /// side decides, so a wide key gets a centred square icon rather than a
    /// stretched one.
    #[must_use]
    pub fn with_icon(mut self, icon: &'static Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Removes or replaces the icon.
    pub fn set_icon(&mut self, icon: Option<&'static Icon>) {
        self.icon = icon;
    }

    /// The shape drawn in place of the label, if any.
    #[inline]
    pub const fn icon(&self) -> Option<&'static Icon> {
        self.icon
    }

    /// A small second label in the top-right corner.
    ///
    /// What a key on a real keyboard has printed above the character it types:
    /// the `!` over the `1`, the `?` over the `+`. It says what the *other*
    /// state of this button would give, which is the whole reason a keyboard
    /// prints it — you cannot discover Shift by pressing Shift if pressing it
    /// is what changes the legend.
    ///
    /// Drawn at two thirds the label's size in the same content colour, so it
    /// reads as an annotation rather than as a second button. Empty is the
    /// normal case and costs nothing.
    #[must_use]
    pub fn with_corner(mut self, corner: impl Into<String>) -> Self {
        self.corner = corner.into();
        self
    }

    /// Replaces the corner label.
    pub fn set_corner(&mut self, corner: impl Into<String>) {
        self.corner = corner.into();
    }

    /// What is printed in the corner, if anything.
    #[inline]
    pub fn corner(&self) -> &str {
        &self.corner
    }

    /// Repeats owed since this was last called, and clears the tally.
    ///
    /// Zero unless [`with_repeat`](Self::with_repeat) was asked for and a finger
    /// has been resting on the button for longer than its delay.
    ///
    /// Reach for [`repeats_pending`](Self::repeats_pending) first when polling
    /// several buttons: taking needs `&mut`, and getting one out of the tree
    /// costs a repaint of the node whether or not anything had changed.
    pub fn take_repeats(&mut self) -> u32 {
        core::mem::take(&mut self.pending)
    }

    /// Reports how long a finger has been resting on it.
    ///
    /// The other half of press-and-hold. [`with_repeat`](Self::with_repeat)
    /// answers "again, and again"; this answers "how long", which is what a
    /// gesture that fires *once* after a delay needs — a key offering its
    /// alternates, a button revealing a menu.
    ///
    /// Costs the same as repeating and no more: the button asks the tree to
    /// wake it only between a press and its release, so a screen nobody is
    /// touching schedules nothing. Read it with [`held_ms`](Self::held_ms).
    #[must_use]
    pub const fn watching_hold(mut self) -> Self {
        self.watches_hold = true;
        self
    }

    /// How long the current press has lasted, in milliseconds.
    ///
    /// `None` when nothing is on it. Updated on each tick while held, so it is
    /// as fresh as the last one — which for a wake-driven tree means as fresh
    /// as whatever asked to be woken.
    ///
    /// A free read: unlike `Ui::widget_mut`, looking does not repaint.
    #[inline]
    pub const fn held_ms(&self) -> Option<u64> {
        if self.held_since.is_some() {
            Some(self.held_ms)
        } else {
            None
        }
    }

    /// Repeats owed, without taking them.
    ///
    /// The read that costs nothing. `Ui::widget_mut` damages the node it hands
    /// out — it cannot know whether the caller changed anything — so polling a
    /// keyboard's sixty keys through it repaints the whole keyboard on every
    /// frame. This is how a caller finds the one key that owes something before
    /// asking for it mutably.
    #[inline]
    pub const fn repeats_pending(&self) -> u32 {
        self.pending
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

    /// The colour role it is drawn in.
    ///
    /// Worth reading before writing: `Ui::widget_mut` repaints whatever it
    /// hands out, so a caller restyling a row of buttons at once should skip
    /// the ones already right.
    #[inline]
    pub const fn role(&self) -> Role {
        self.role
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
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        let radius = ctx.theme.radius(self.radius);
        let (background, content) = interactive_pair(ctx.theme, self.role, ctx.state);
        canvas.fill_rounded_rect(ctx.bounds, radius, background);
        if ctx.state.contains(VisualState::FOCUSED) {
            focus_ring(ctx.theme, ctx.bounds, radius, canvas);
        }
        // The corner first, so that a label wide enough to reach it is drawn
        // over the annotation rather than under it — the character somebody is
        // about to type matters more than the one they are not.
        if !self.corner.is_empty() {
            let size = (i32::from(self.style.size_px) * 2 / 3).max(1) as u16;
            let small = TextStyle {
                size_px: size,
                ..self.style
            };
            let pad = i32::from(size) / 3;
            let corner = Rect::new(
                ctx.bounds.x,
                ctx.bounds.y + pad,
                (ctx.bounds.width - pad).max(0),
                i32::from(size) + pad,
            );
            draw_aligned(
                canvas,
                ctx.text,
                small,
                corner,
                (Align::End, Align::Start),
                &self.corner,
                content,
            );
        }
        // An icon takes the label's place. Square and centred, from the shorter
        // side, so a wide key gets a centred picture rather than a stretched
        // one; the corner legend above is untouched, since a key can have both.
        if let Some(icon) = self.icon {
            let side = (ctx.bounds.width.min(ctx.bounds.height) * ICON_SHARE / 100).max(1);
            let box_ = Rect::new(
                ctx.bounds.x + (ctx.bounds.width - side) / 2,
                ctx.bounds.y + (ctx.bounds.height - side) / 2,
                side,
                side,
            );
            canvas.draw_icon(icon, box_, content, background);
            return;
        }

        // The label sits low when something shares the key with it, so the two
        // do not collide and the row of characters still reads as a row.
        let vertical = if self.corner.is_empty() {
            Align::Center
        } else {
            Align::End
        };
        let inset = if self.corner.is_empty() {
            ctx.bounds
        } else {
            let pad = i32::from(self.style.size_px) / 4;
            Rect::new(
                ctx.bounds.x,
                ctx.bounds.y,
                ctx.bounds.width,
                (ctx.bounds.height - pad).max(0),
            )
        };
        draw_aligned(
            canvas,
            ctx.text,
            self.style,
            inset,
            (Align::Center, vertical),
            &self.label,
            content,
        );
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        // A repeating button lives on the down edge instead of the up one, and
        // has to, since the repeats begin while the finger is still there.
        if self.repeat.is_some() || self.watches_hold {
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
                    self.held_ms = 0;
                    // The wake that carries the hold. Asked for here and given
                    // back by `animate` the moment the finger leaves, so a tree
                    // nobody is touching schedules nothing.
                    ctx.request_animation();
                    // A repeating button acts on the press; one that merely
                    // watches the hold does not, so that it still emits on
                    // release the way every other button does — the gesture is
                    // "hold for something else", not "act sooner".
                    if self.repeat.is_some()
                        && let Some(message) = self.message.clone()
                    {
                        ctx.emit(message);
                    }
                    return if self.repeat.is_some() {
                        Handled::Yes
                    } else {
                        Handled::No
                    };
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
                    self.held_ms = 0;
                    // Whatever was owed is dropped with the press: a repeat
                    // nobody collected before the finger lifted is one the user
                    // did not wait for.
                    self.pending = 0;
                    // A repeating button has already acted; one that only
                    // watches must fall through, or it would never emit at all.
                    if self.repeat.is_some() {
                        return Handled::Yes;
                    }
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
        let Some(since) = self.held_since else {
            return Animation::NONE;
        };
        self.held_ms = now_ms.saturating_sub(since);
        let Some(repeat) = self.repeat else {
            // Watching the hold and nothing else: come back at the rate the
            // tree animates at, which is what makes `held_ms` current without
            // this widget inventing a cadence of its own.
            return Animation {
                repaint: false,
                next: crate::Wake::Animating,
            };
        };
        let held = self.held_ms;
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

impl<M> Describe for Button<M> {
    const KIND: &'static str = "button";

    const PROPERTIES: &'static [Property] = &[
        Property::new("text", PropertyKind::Text, "The label."),
        Property::new(
            "on-press",
            PropertyKind::Message(Payload::None),
            "The message emitted on activation. Omitted, the button is inert — it draws and does not emit.",
        ),
        Property::new(
            "role",
            PropertyKind::Enum(ROLES),
            "Colour role. The label's colour comes from the theme's pairing, so it stays readable whichever role is chosen.",
        ),
        Property::new(
            "radius",
            PropertyKind::Enum(RADII),
            "Corner rounding token. The theme decides the pixels.",
        ),
        Property::new(
            "size",
            PropertyKind::Int { min: 6, max: 96 },
            "Text size in logical pixels.",
        )
        .in_pixels(),
        Property::new(
            "corner",
            PropertyKind::Text,
            "A small legend in the corner, as the keyboard's globe key carries its layout.",
        ),
        Property::new(
            "no-focus",
            PropertyKind::Bool,
            "The button never takes focus, so pressing it does not steal the caret from a field.",
        ),
        Property::new(
            "repeat-delay",
            PropertyKind::Int {
                min: 100,
                max: 2000,
            },
            "Milliseconds held before the press repeats. Set without `repeat-interval`, the interval becomes this same value, so a lone half of the pair is a button that repeats steadily rather than one that ignores the setting.",
        ),
        Property::new(
            "repeat-interval",
            PropertyKind::Int { min: 10, max: 1000 },
            "Milliseconds between repeats. Set without `repeat-delay`, the delay becomes this same value, by the rule `repeat-delay` describes.",
        ),
        Property::new(
            "watch-hold",
            PropertyKind::Bool,
            "Report how long the button has been held, for a long-press.",
        ),
    ];

    fn get(&self, name: &str) -> Option<Value> {
        Some(match name {
            "text" => Value::text(self.label.as_str()),
            // The message is the application's, and this crate has never seen
            // its type. See the `describe` module docs.
            "on-press" => return None,
            "role" => Value::role(self.role),
            "radius" => Value::radius(self.radius),
            "size" => Value::Int(i32::from(self.style.size_px)),
            "corner" => Value::text(self.corner.as_str()),
            "no-focus" => Value::Bool(self.no_focus),
            // One field carries both halves, so a button that does not repeat
            // reports neither rather than reporting a schedule it does not have.
            "repeat-delay" => Value::Int(millis(self.repeat?.delay_ms)),
            "repeat-interval" => Value::Int(millis(self.repeat?.interval_ms)),
            "watch-hold" => Value::Bool(self.watches_hold),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            "text" => self.label = value.as_text()?,
            "on-press" => return Err(Mismatch::Supplied),
            "role" => self.role = value.as_role()?,
            "radius" => self.radius = value.as_radius()?,
            "size" => self.style.size_px = value.as_size()?,
            "corner" => self.corner = value.as_text()?,
            "no-focus" => self.no_focus = value.as_bool()?,
            // The two halves arrive one property at a time, and either may be
            // the only one a file mentions. Whichever comes first supplies the
            // other, so a lone `repeat-delay=400` is a button that repeats every
            // 400 ms after 400 ms rather than one that quietly does not repeat.
            "repeat-delay" => {
                let delay_ms = value.as_millis()?;
                self.repeat = Some(match self.repeat {
                    Some(repeat) => Repeat { delay_ms, ..repeat },
                    None => Repeat {
                        delay_ms,
                        interval_ms: delay_ms.max(1),
                    },
                });
            }
            "repeat-interval" => {
                // Never zero, exactly as `with_repeat` insists: an interval of
                // nothing is a repeat every frame forever.
                let interval_ms = value.as_millis()?.max(1);
                self.repeat = Some(match self.repeat {
                    Some(repeat) => Repeat {
                        interval_ms,
                        ..repeat
                    },
                    None => Repeat {
                        delay_ms: interval_ms,
                        interval_ms,
                    },
                });
            }
            "watch-hold" => self.watches_hold = value.as_bool()?,
            _ => return Err(Mismatch::Unknown),
        }
        Ok(())
    }
}

/// A duration reported to an inspector, saturating rather than wrapping.
///
/// The schedule is `u64` because a clock is; an editor's spinbox is not, and a
/// delay of half a million years is not worth a wider `Value` variant.
fn millis(ms: u64) -> i32 {
    i32::try_from(ms).unwrap_or(i32::MAX)
}
