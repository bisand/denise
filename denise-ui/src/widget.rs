//! What a widget is, and the two contexts it is handed.
//!
//! A widget owns its own state and knows how to draw itself inside a rectangle it
//! is given. It does not own its position, its children, its z-order or its
//! damage — the tree owns those, which is what keeps the invalidation rules in one
//! place instead of scattered across every widget.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::any::Any;

use denise::{InputEvent, Rect, Size, Theme};
use denise_render::Canvas;
use denise_text::TextEngine;

use crate::motion::Wake;
use crate::widgets::describe::DynDescribe;

/// Upcast to [`Any`], so an application can get its concrete widget type back out
/// of the tree. Blanket-implemented; never implement it by hand.
pub trait AsAny: 'static {
    /// Borrows as `dyn Any`.
    fn as_any(&self) -> &dyn Any;
    /// Mutably borrows as `dyn Any`.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Any> AsAny for T {
    #[inline]
    fn as_any(&self) -> &dyn Any {
        self
    }
    #[inline]
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Visual state the tree tracks on the widget's behalf.
///
/// Widgets do not track hover or press themselves. The tree does, and it marks the
/// node dirty when any of these change — which is the whole reason a stale-pixel
/// bug cannot come from a widget forgetting to invalidate on hover.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct VisualState(u8);

impl VisualState {
    /// Nothing set.
    pub const NONE: Self = Self(0);
    /// The pointer is over this widget.
    pub const HOVERED: Self = Self(1 << 0);
    /// A pointer button went down on this widget and has not come up.
    pub const PRESSED: Self = Self(1 << 1);
    /// This widget has keyboard focus.
    pub const FOCUSED: Self = Self(1 << 2);
    /// This widget, or an ancestor, is disabled.
    pub const DISABLED: Self = Self(1 << 3);

    /// Returns `true` if every bit in `other` is set.
    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Sets or clears `other`.
    #[inline]
    pub const fn set(self, other: Self, on: bool) -> Self {
        Self(if on {
            self.0 | other.0
        } else {
            self.0 & !other.0
        })
    }

    /// Returns `true` if no bit is set.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl core::ops::BitOr for VisualState {
    type Output = Self;
    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// Whether an event was consumed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Handled {
    /// The widget ignored the event.
    #[default]
    No,
    /// The widget acted on the event.
    ///
    /// **This also marks the node dirty.** A widget that consumes an event has
    /// almost always changed what it draws, and the cost of being wrong is
    /// repainting one widget-sized rectangle. Missing an invalidation costs a
    /// stale frame that only shows up on hardware, so the default errs the cheap
    /// way. Use [`EventCtx::invalidate`] for the rare change that consumes nothing.
    Yes,
}

impl Handled {
    /// Returns `true` for [`Handled::Yes`].
    #[inline]
    pub const fn is_handled(self) -> bool {
        matches!(self, Handled::Yes)
    }
}

/// Something a widget is asked to react to.
#[derive(Debug)]
#[non_exhaustive]
pub enum Event<'a> {
    /// Raw input, already routed to this widget by hit test or focus.
    Input(&'a InputEvent),
    /// This widget just took keyboard focus.
    FocusGained,
    /// This widget just lost keyboard focus.
    FocusLost,
    /// A press this widget was holding ended without a release.
    ///
    /// The tree drops a held press whenever the thing being pressed stops being
    /// reachable — a scene pushed over it, the scene it lives in popped, its
    /// node removed or disabled — and no pointer event describes that. A widget
    /// that only tracks Down and Up would go on believing a finger is still
    /// resting on it, which for anything driving a timer from that belief means
    /// waking a panel that nobody is touching.
    ///
    /// Ordinary widgets need not handle it: the visual pressed state is cleared
    /// by the tree either way.
    PressCancelled,
}

/// What a caller can promise a widget about the space it will get.
///
/// Not a constraint in the layout-engine sense: there is no minimum, no maximum
/// and nothing to satisfy. It is the one fact a widget may need in order to
/// answer at all — an [`Alert`](crate::widgets::Alert) has no height until it
/// knows the width its text wraps to, and a [`Rating`](crate::widgets::Rating)
/// has no width until it knows how tall its stars are.
///
/// `None` on an axis means the caller cannot promise anything there, which is
/// the usual case and the default.
///
/// ```
/// # use denise_ui::Offer;
/// // "You will get 300 pixels of width; the height is up to you."
/// let offer = Offer::wide(300);
/// assert_eq!(offer.width, Some(300));
/// assert_eq!(offer.height, None);
///
/// assert_eq!(Offer::NOTHING, Offer::default());
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Offer {
    /// The width the caller can promise, if it can promise one.
    pub width: Option<i32>,
    /// The height, likewise.
    pub height: Option<i32>,
}

impl Offer {
    /// Nothing promised on either axis.
    pub const NOTHING: Self = Self {
        width: None,
        height: None,
    };

    /// This much width, and nothing said about the height.
    #[must_use]
    pub const fn wide(width: i32) -> Self {
        Self {
            width: Some(width),
            height: None,
        }
    }

    /// This much height, and nothing said about the width.
    #[must_use]
    pub const fn tall(height: i32) -> Self {
        Self {
            width: None,
            height: Some(height),
        }
    }

    /// Both axes promised.
    #[must_use]
    pub const fn exactly(size: Size) -> Self {
        Self {
            width: Some(size.width as i32),
            height: Some(size.height as i32),
        }
    }
}

/// What a widget would like to be, on the axes it has an opinion about.
///
/// **Per axis, and both optional**, because that is what the widgets actually
/// are. A [`List`](crate::widgets::List) has an opinion about both. An
/// [`Alert`](crate::widgets::Alert) has one about its height and none about its
/// width — it is a banner, and a banner is as wide as you make it. A
/// [`Panel`](crate::widgets::Panel) has none about either, because it is the
/// background other things sit on and has no content of its own.
///
/// A single `Option<Size>` would force a widget with an opinion about one axis
/// to invent one about the other, and an invented size is worse than no size:
/// no size, the caller notices and decides.
///
/// ```
/// # use denise_ui::Measured;
/// assert_eq!(Measured::NOTHING, Measured::default());
/// assert_eq!(Measured::wide(120).width, Some(120));
/// assert_eq!(Measured::wide(120).height, None);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Measured {
    /// The width it would like, if it has a view.
    pub width: Option<i32>,
    /// The height it would like, if it has a view.
    pub height: Option<i32>,
}

impl Measured {
    /// No opinion on either axis. The default, and what most widgets answer.
    pub const NOTHING: Self = Self {
        width: None,
        height: None,
    };

    /// An opinion about the width only.
    #[must_use]
    pub const fn wide(width: i32) -> Self {
        Self {
            width: Some(width),
            height: None,
        }
    }

    /// An opinion about the height only.
    #[must_use]
    pub const fn tall(height: i32) -> Self {
        Self {
            width: None,
            height: Some(height),
        }
    }

    /// An opinion about both.
    #[must_use]
    pub const fn both(width: i32, height: i32) -> Self {
        Self {
            width: Some(width),
            height: Some(height),
        }
    }
}

/// What a widget may consult while measuring itself.
///
/// The theme and the fonts, and deliberately nothing else. No bounds, because a
/// widget being asked how big it wants to be must not answer with how big it
/// currently is; no state, because a hovered button is not a wider button; no
/// clock, because a size that changed every frame would be a layout that never
/// settled.
#[derive(Debug)]
pub struct MeasureCtx<'a> {
    /// The active theme. Sizes come from its metrics.
    pub theme: &'a Theme,
    /// Fonts and the glyph cache.
    pub text: &'a mut TextEngine,
}

/// What a widget needs in order to draw itself.
///
/// Mutable, because [`PaintCtx::text`] is: measuring a string is what fills the
/// glyph cache, so a widget that measures and then draws rasterises each glyph
/// once rather than twice. The widget itself is still `&self`.
#[derive(Debug)]
pub struct PaintCtx<'a> {
    /// The active theme. Widgets name roles, never colours.
    pub theme: &'a Theme,
    /// Fonts and the glyph cache.
    pub text: &'a mut TextEngine,
    /// Absolute bounds of this widget, in surface pixels.
    pub bounds: Rect,
    /// Hover, press, focus and enabled state, tracked by the tree.
    pub state: VisualState,
    /// Milliseconds from an arbitrary epoch, as last given to [`crate::Ui::tick`].
    pub now_ms: u64,
}

/// What a widget can do while handling an event.
#[derive(Debug)]
pub struct EventCtx<'a, M> {
    /// Absolute bounds of this widget, in surface pixels.
    pub bounds: Rect,
    /// The active theme.
    pub theme: &'a Theme,
    /// Fonts and the glyph cache.
    ///
    /// Needed while *handling* events, not only while painting: a text field with
    /// a proportional font cannot work out where its caret is without measuring
    /// the text in front of it.
    pub text: &'a mut TextEngine,
    /// Hover, press, focus and enabled state.
    pub state: VisualState,
    /// Milliseconds from an arbitrary epoch.
    pub now_ms: u64,
    messages: &'a mut Vec<M>,
    dirty: bool,
    wants_focus: bool,
    wants_animation: bool,
    reveal: Option<Rect>,
    resize: Option<(i32, u64)>,
}

impl<'a, M> EventCtx<'a, M> {
    pub(crate) fn new(
        bounds: Rect,
        theme: &'a Theme,
        text: &'a mut TextEngine,
        state: VisualState,
        now_ms: u64,
        messages: &'a mut Vec<M>,
    ) -> Self {
        Self {
            bounds,
            theme,
            text,
            state,
            now_ms,
            messages,
            dirty: false,
            wants_focus: false,
            wants_animation: false,
            reveal: None,
            resize: None,
        }
    }

    /// Queues a message for the application to pick up with
    /// [`Ui::drain_messages`](crate::Ui::drain_messages).
    ///
    /// This is the whole event-handling story: no callbacks, no `Rc<RefCell<_>>`,
    /// no widget holding a reference to another widget. The application dispatches
    /// centrally, where it can see all of its own state.
    #[inline]
    pub fn emit(&mut self, message: M) {
        self.messages.push(message);
    }

    /// Marks this widget's rectangle for repaint.
    ///
    /// Rarely needed: returning [`Handled::Yes`] already does it.
    #[inline]
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// Asks the tree to move keyboard focus here.
    #[inline]
    pub fn request_focus(&mut self) {
        self.wants_focus = true;
    }

    /// Asks the tree to start calling [`Widget::animate`] on this widget.
    ///
    /// Called at the moment the widget starts needing frames — a knob that just
    /// began sliding, a caret whose field just took focus. The calls continue
    /// until `animate` answers `next_ms: None`, which is the widget saying it
    /// has arrived. See [`Widget::animate`] for what that hand-back means on a
    /// device that is supposed to spend its day asleep.
    #[inline]
    pub fn request_animation(&mut self) {
        self.wants_animation = true;
    }

    /// Asks the tree to scroll `rect` — in absolute surface coordinates, like
    /// [`EventCtx::bounds`] — into view in every scrollable ancestor.
    ///
    /// For a widget whose *interior* moves: a list whose selection walked below
    /// the fold reveals the selected row's rectangle, and the viewport follows
    /// the selection the way it follows focus. Widgets that are themselves the
    /// focus target need nothing — focus already reveals.
    #[inline]
    pub fn reveal(&mut self, rect: Rect) {
        self.reveal = Some(rect);
    }

    /// Asks the tree to carry this node's **height** to `height` over
    /// `duration_ms`, through the same tween
    /// [`Ui::animate_layout`](crate::Ui::animate_layout) drives.
    ///
    /// A widget does not own its geometry — the tree does — which is why this is
    /// a request rather than a call, and why it sits beside
    /// [`reveal`](EventCtx::reveal) rather than anywhere else: those are the two
    /// things a widget can want and cannot do.
    ///
    /// Reach for it only where nothing else can act. The one use in this crate
    /// is a [`Collapse`](crate::widgets::Collapse) with no message: a section
    /// that reports its toggles is telling the application to drive the fold,
    /// and one that reports nothing has nobody else to.
    pub fn resize_height(&mut self, height: i32, duration_ms: u64) {
        self.resize = Some((height, duration_ms));
    }

    pub(crate) fn finish(self) -> Outcome {
        Outcome {
            dirty: self.dirty,
            wants_focus: self.wants_focus,
            wants_animation: self.wants_animation,
            reveal: self.reveal,
            resize: self.resize,
        }
    }
}

/// What handling one event left the tree to do.
///
/// Grown from a tuple once it reached five fields, which is the point at which
/// `(bool, bool, bool, Option<Rect>, Option<(i32, u64)>)` stops being readable
/// at the call site.
pub(crate) struct Outcome {
    pub(crate) dirty: bool,
    pub(crate) wants_focus: bool,
    pub(crate) wants_animation: bool,
    pub(crate) reveal: Option<Rect>,
    pub(crate) resize: Option<(i32, u64)>,
}

/// What a widget reports back after [`Widget::animate`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Animation {
    /// `true` if the widget's appearance changed and it needs repainting.
    pub repaint: bool,
    /// When the widget wants to be asked again — at the tree's rate, at a
    /// particular time, or never.
    pub next: Wake,
}

impl Animation {
    /// Nothing is animating.
    pub const NONE: Self = Self {
        repaint: false,
        next: Wake::Never,
    };

    /// Moving: repaint, and come back at the tree's animation rate.
    ///
    /// The answer nearly every mid-transition widget wants, and the reason none
    /// of them carries a frame-rate constant any more — how fast "the tree's
    /// rate" is belongs to [`Motion`](crate::Motion).
    pub const MOVING: Self = Self {
        repaint: true,
        next: Wake::Animating,
    };

    /// Waiting for a deadline, with nothing to repaint until it arrives.
    ///
    /// The toast arrangement: one wake at the moment something is due, rather
    /// than a frame a tick spent noticing that it is not due yet.
    #[inline]
    pub const fn due_at(due_ms: u64) -> Self {
        Self {
            repaint: false,
            next: Wake::At(due_ms),
        }
    }
}

/// A thing that draws itself in a rectangle and reacts to input.
///
/// `M` is the application's message type. A widget never calls back into the
/// application; it emits an `M` and the application decides what that means.
pub trait Widget<M>: AsAny {
    /// Draws into `canvas`, which is already clipped to this widget's bounds
    /// intersected with the damage region being repainted.
    ///
    /// Paint as though the whole widget were visible. The clip turns that into an
    /// incremental repaint, so there is never a second draw path to keep in step
    /// with the first.
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>);

    /// Reacts to an event routed to this widget.
    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        let _ = (event, ctx);
        Handled::No
    }

    /// How big this widget would like to be, given what the caller can promise.
    ///
    /// [`Measured::NOTHING`] — the default — means no opinion, which is the
    /// honest answer for a [`Panel`], an [`Image`] or a [`Video`]: they are
    /// whatever rectangle they are given.
    ///
    /// **The tree never calls this.** It is a query, offered for a caller that
    /// is not holding the concrete widget and so cannot reach its inherent
    /// `preferred_width`/`preferred_height`. That distinction is the line
    /// between this toolkit and a layout engine, and it survives: an
    /// intrinsic-size *protocol* is one where the tree asks every widget how big
    /// it wants to be and then places it, and nothing in this crate consumes
    /// this. See `docs/design.md`, and `docs/arrange.md` for the caller it was
    /// written for.
    ///
    /// [`Panel`]: crate::widgets::Panel
    /// [`Image`]: crate::widgets::Image
    /// [`Video`]: crate::widgets::Video
    fn measure(&self, ctx: &mut MeasureCtx<'_>, offered: Offer) -> Measured {
        let _ = (ctx, offered);
        Measured::NOTHING
    }

    /// Returns `true` if the pointer can hit this widget.
    ///
    /// Non-interactive widgets are invisible to hit testing, so a [`Label`] inside
    /// a [`Button`] does not swallow the click — the button is still the topmost
    /// hittable node under the pointer.
    ///
    /// [`Label`]: crate::widgets::Label
    /// [`Button`]: crate::widgets::Button
    fn accepts_pointer(&self) -> bool {
        false
    }

    /// Returns `true` if this widget can take keyboard focus.
    fn focusable(&self) -> bool {
        false
    }

    /// Returns `true` if a press on this widget should leave focus exactly where
    /// it is.
    ///
    /// Not the same as being unfocusable, and the difference is the whole point.
    /// Pressing an ordinary non-focusable node — a [`Label`], a [`Panel`] — drops
    /// focus, which is what makes a text field commit and stop blinking when the
    /// user clicks away. A widget answering `true` here is asking for neither: it
    /// takes no focus *and* costs none.
    ///
    /// An on-screen keyboard's keys are the reason this exists. A key is pressed
    /// while a field is being typed into, and both the ordinary answers are wrong
    /// — taking focus blurs the field, and dropping focus blurs it too.
    ///
    /// [`Label`]: crate::widgets::Label
    /// [`Panel`]: crate::widgets::Panel
    fn preserves_focus(&self) -> bool {
        false
    }

    /// Advances time-based state. Called only while this widget has asked to
    /// animate — see [`EventCtx::request_animation`] — and stops being called
    /// the moment it answers [`Wake::Never`].
    ///
    /// The contract is deliberate about who holds the responsibility: **the
    /// widget keeps itself animating, and must stop asking.** On a panel that
    /// spends its day idle, a bounded transition — a knob crossing, a toast
    /// fading — costs its duration and then hands the CPU back. An animation
    /// that never answers [`Wake::Never`] keeps the device awake for as long as
    /// its node is visible, which is legitimate for a spinner and ruinous for
    /// anything that merely forgot. [`Ui::animating`](crate::Ui::animating)
    /// exists so a test can prove a tree at rest holds nobody awake.
    ///
    /// May be called earlier than the time it asked for: the tree wakes for the
    /// most impatient animation and asks everybody. Answer honestly for the
    /// clock given and it comes out right.
    ///
    /// # Say what kind of waiting it is
    ///
    /// A widget that is *moving* answers [`Wake::Animating`] and lets
    /// [`Motion`](crate::Motion) decide how often that is — one setting, tree
    /// wide, which a widget with its own frame-rate constant would opt out of
    /// without meaning to. A widget waiting for something to *happen* answers
    /// [`Wake::At`] with the time, and the rate never touches it.
    fn animate(&mut self, now_ms: u64) -> Animation {
        let _ = now_ms;
        Animation::NONE
    }

    /// Lands whatever is in flight at its end state, without animating it.
    ///
    /// Called instead of [`animate`](Widget::animate) while the tree's
    /// [`Motion`](crate::Motion) is [`Motion::None`](crate::Motion::None) —
    /// reduced motion, or a power budget with no room for movement. A knob
    /// arrives, a slide is over, a fade is not a fade.
    ///
    /// The return is an ordinary [`Animation`], so a widget that has a
    /// **schedule** as well as a motion keeps it: a carousel that lands its
    /// slide instantly still answers [`Wake::At`] for its auto-advance, because
    /// turning motion off is not the same as stopping the clock. [`Wake::
    /// Animating`](Wake::Animating) is the one answer that means nothing here —
    /// there is no rate to come back at — and the tree reads it as
    /// [`Wake::Never`].
    ///
    /// The default settles nothing and asks for nothing, which is right for the
    /// widgets that do not animate and for unbounded ones like a spinner: under
    /// `Motion::None` a spinner simply does not turn.
    fn snap(&mut self, now_ms: u64) -> Animation {
        let _ = now_ms;
        Animation::NONE
    }

    /// This widget's property description, if it has one.
    ///
    /// The tree stores widgets boxed, and [`Describe`](crate::widgets::Describe)
    /// has associated constants, so it is not object-safe and cannot be reached
    /// through a `dyn Widget<M>` directly. These two hand over the object-safe
    /// half, and they are what
    /// [`Ui::set_property`](crate::Ui::set_property) calls.
    ///
    /// Opting in is two lines in a widget's `Widget` implementation, once it
    /// implements `Describe`:
    ///
    /// ```ignore
    /// fn describe(&self) -> Option<&dyn DynDescribe> { Some(self) }
    /// fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> { Some(self) }
    /// ```
    ///
    /// The default answers `None`, so a one-off widget in an application is not
    /// obliged to describe itself — it simply does not appear in a form file or
    /// a property inspector, which for a widget nobody else will ever place is
    /// the right amount of ceremony. Every widget this crate ships does opt in.
    fn describe(&self) -> Option<&dyn DynDescribe> {
        None
    }

    /// The mutable half of [`describe`](Widget::describe).
    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        None
    }
}

/// A widget that draws nothing and hits nothing.
///
/// Used for scene roots and for grouping: a node exists to position and clip its
/// children, and does not have to paint to do that.
#[derive(Clone, Copy, Debug, Default)]
pub struct Void;

impl<M: 'static> Widget<M> for Void {
    fn paint(&self, _ctx: &mut PaintCtx<'_>, _canvas: &mut Canvas<'_>) {}
}

pub(crate) type BoxedWidget<M> = Box<dyn Widget<M>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visual_state_bits() {
        let s = VisualState::HOVERED | VisualState::FOCUSED;
        assert!(s.contains(VisualState::HOVERED));
        assert!(!s.contains(VisualState::PRESSED));
        assert!(
            s.set(VisualState::HOVERED, false)
                .contains(VisualState::FOCUSED)
        );
        assert!(VisualState::NONE.is_empty());
    }
}
