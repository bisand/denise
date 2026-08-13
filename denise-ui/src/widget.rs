//! What a widget is, and the two contexts it is handed.
//!
//! A widget owns its own state and knows how to draw itself inside a rectangle it
//! is given. It does not own its position, its children, its z-order or its
//! damage — the tree owns those, which is what keeps the invalidation rules in one
//! place instead of scattered across every widget.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::any::Any;

use denise::{InputEvent, Rect, Theme};
use denise_render::Canvas;
use denise_text::TextEngine;

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

    pub(crate) fn finish(self) -> (bool, bool, bool) {
        (self.dirty, self.wants_focus, self.wants_animation)
    }
}

/// What a widget reports back after [`Widget::animate`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Animation {
    /// `true` if the widget's appearance changed and it needs repainting.
    pub repaint: bool,
    /// When the widget wants to be asked again, in the same clock as `now_ms`.
    pub next_ms: Option<u64>,
}

impl Animation {
    /// Nothing is animating.
    pub const NONE: Self = Self {
        repaint: false,
        next_ms: None,
    };
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

    /// Advances time-based state. Called only while this widget has asked to
    /// animate — see [`EventCtx::request_animation`] — and stops being called
    /// the moment it answers `next_ms: None`.
    ///
    /// The contract is deliberate about who holds the responsibility: **the
    /// widget keeps itself animating, and must stop asking.** On a panel that
    /// spends its day idle, a bounded transition — a knob crossing, a toast
    /// fading — costs its duration and then hands the CPU back. An animation
    /// that never answers `None` keeps the device awake for as long as its node
    /// is visible, which is legitimate for a spinner and ruinous for anything
    /// that merely forgot. [`Ui::animating`](crate::Ui::animating) exists so a
    /// test can prove a tree at rest holds nobody awake.
    ///
    /// May be called earlier than the time it asked for: the tree wakes for the
    /// most impatient animation and asks everybody. Answer honestly for the
    /// clock given and it comes out right.
    fn animate(&mut self, now_ms: u64) -> Animation {
        let _ = now_ms;
        Animation::NONE
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
