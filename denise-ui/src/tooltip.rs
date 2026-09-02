//! The pointer-rested-here bubble, and the state machine behind it.
//!
//! A tooltip is not a widget and not a node. Everything hard about one happens
//! *before* it would exist: the dwell timer has nothing to belong to yet, the
//! placement needs another node's bounds, and it disappears when the pointer
//! leaves the **anchor** rather than the bubble — which the pointer must never
//! be able to enter at all.
//!
//! So it lives beside the cursor sprite: something [`Ui`](crate::Ui) draws over
//! everything because it is not part of the tree. This module holds the timing
//! rules and the drawing; `Ui` holds the hover it watches.
//!
//! # It is a pointer affordance
//!
//! Tooltips need hover, and a touchscreen has none. On a touch-only panel this
//! does nothing whatever, and that is the honest outcome rather than a gap: the
//! panels that want tooltips are the mouse-driven industrial HMIs, and the
//! `denise-win32`, `denise-macos` and `denise-activex` controls embedded in
//! desktop applications where every other control has one.

use alloc::string::String;

use denise::{Point, Radius, Rect, Role, Size, Theme};
use denise_render::Pen;
use denise_text::{TextEngine, TextStyle};

use crate::overlay::{Side, anchored};

/// How long the pointer must rest before a tooltip appears.
///
/// Long enough that moving across a toolbar does not trail bubbles behind the
/// pointer, short enough that somebody who stopped to read gets an answer. The
/// usual desktop figure, and the one a person's expectations were set by.
const DELAY_MS: u64 = 600;

/// Space between the bubble and its anchor.
const GAP: i32 = 6;

/// Space between the text and the bubble's edge.
const PADDING: i32 = 6;

/// What the tree is doing about tooltips at this instant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Nothing hovered that has one.
    Idle,
    /// The pointer is resting on a node with a tooltip; it appears at this time.
    Waiting { due_ms: u64 },
    /// Showing.
    Shown,
}

/// The tooltip state machine, owned by [`Ui`](crate::Ui).
#[derive(Clone, Debug)]
pub(crate) struct Tooltip {
    pub(crate) phase: Phase,
    /// The text and the rectangle it was placed against, captured when the
    /// bubble appeared. Kept rather than looked up each frame so a tooltip
    /// cannot outlive its node into a panic.
    shown: Option<(String, Rect)>,
    style: TextStyle,
}

impl Tooltip {
    pub(crate) fn new() -> Self {
        Self {
            phase: Phase::Idle,
            shown: None,
            style: TextStyle::built_in(14),
        }
    }

    /// Sets the size the text is drawn at. See
    /// [`Ui::set_tooltip_size`](crate::Ui::set_tooltip_size).
    ///
    /// Never zero: a tooltip that measures nothing is a bubble with no text in
    /// it, which is worse than one at the wrong size.
    pub(crate) fn set_size(&mut self, size_px: u16) {
        self.style.size_px = size_px.max(1);
    }

    /// Whether a bubble is on screen right now.
    ///
    /// The caller damages what it covers **before** changing anything, because
    /// every state change here forgets where the bubble was — which is the bug
    /// the damage test caught: the footprint has to be measured while it is
    /// still known.
    #[inline]
    pub(crate) fn is_shown(&self) -> bool {
        self.phase == Phase::Shown
    }

    /// The pointer moved onto something — or onto nothing.
    ///
    /// Returns `true` if a visible bubble was taken away.
    pub(crate) fn hover_changed(&mut self, has_tooltip: bool, now_ms: u64) -> bool {
        let was_shown = self.phase == Phase::Shown;
        self.phase = if has_tooltip {
            Phase::Waiting {
                due_ms: now_ms.saturating_add(DELAY_MS),
            }
        } else {
            Phase::Idle
        };
        if was_shown {
            self.shown = None;
        }
        was_shown
    }

    /// Anything that means the person has moved on: a press, a key, focus
    /// moving. Returns `true` if a visible bubble was taken away.
    pub(crate) fn dismiss(&mut self) -> bool {
        let was_shown = self.phase == Phase::Shown;
        self.phase = Phase::Idle;
        self.shown = None;
        was_shown
    }

    /// Whether this event means the person has moved on.
    ///
    /// A press or a key does; a pointer move does not, because moving *is* how
    /// somebody arrives at the thing they are about to rest on. Scroll counts:
    /// what was under the pointer is no longer what it was.
    pub(crate) fn dismiss_wanted(&self, event: &denise::InputEvent) -> bool {
        use denise::InputEvent;
        self.phase != Phase::Idle
            && matches!(
                event,
                InputEvent::PointerButton { .. }
                    | InputEvent::TouchDown { .. }
                    | InputEvent::Key { .. }
                    | InputEvent::Text { .. }
                    | InputEvent::PointerScroll { .. }
                    | InputEvent::PointerLeft
                    | InputEvent::SurfaceResized { .. }
            )
    }

    /// When the tree should be woken for this tooltip, if ever.
    ///
    /// **The coupling that makes the feature work.** A kiosk blocks on input
    /// until the tree says it wants waking; if only animations reported
    /// deadlines then the pointer would rest, nothing would wake, and the
    /// bubble would appear the next time something unrelated happened. Which
    /// looks like a bug, and is one.
    #[inline]
    pub(crate) fn next_wake(&self) -> Option<u64> {
        match self.phase {
            Phase::Waiting { due_ms } => Some(due_ms),
            _ => None,
        }
    }

    /// Advances the timer. Returns `true` if the bubble just appeared.
    pub(crate) fn tick(&mut self, now_ms: u64, text: Option<&str>, anchor: Rect) -> bool {
        let Phase::Waiting { due_ms } = self.phase else {
            return false;
        };
        if now_ms < due_ms {
            return false;
        }
        let Some(text) = text.filter(|text| !text.is_empty()) else {
            // The node lost its tooltip while the pointer rested on it.
            self.phase = Phase::Idle;
            return false;
        };
        self.phase = Phase::Shown;
        self.shown = Some((String::from(text), anchor));
        true
    }

    /// Where the bubble is, if one is showing.
    pub(crate) fn bounds(&self, surface: Size, engine: &mut TextEngine) -> Option<Rect> {
        let (text, anchor) = self.shown.as_ref()?;
        Some(place(surface, *anchor, text, self.style, engine))
    }

    /// Draws the bubble, if one is showing.
    pub(crate) fn paint(
        &self,
        theme: &Theme,
        surface: Size,
        engine: &mut TextEngine,
        canvas: &mut Pen<'_>,
    ) {
        let Some((text, anchor)) = self.shown.as_ref() else {
            return;
        };
        let bounds = place(surface, *anchor, text, self.style, engine);
        if bounds.is_empty() {
            return;
        }
        // `Neutral` and its own content: a tooltip is a small saturated surface
        // over arbitrary content, so it needs a pairing of its own rather than
        // to borrow the panel's — the rule every widget here follows.
        let (fill, content) = theme.pair(Role::Neutral);
        canvas.fill_rounded_rect(bounds, theme.radius(Radius::Field), fill);
        engine.draw(
            canvas,
            self.style,
            Point::new(bounds.x + PADDING, bounds.y + PADDING),
            text,
            content,
        );
    }
}

/// Where a bubble of `text` sits against `anchor`.
///
/// Below by default, flipping above near the bottom edge, through the same
/// [`anchored`] every popup uses — so a tooltip and a dropdown near the same
/// screen edge behave the same way.
fn place(
    surface: Size,
    anchor: Rect,
    text: &str,
    style: TextStyle,
    engine: &mut TextEngine,
) -> Rect {
    let extent = engine.measure(style, text);
    let size = Size::new(
        extent.width + PADDING as u32 * 2,
        extent.height + PADDING as u32 * 2,
    );
    anchored(surface, anchor, size, Side::Below, GAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> TextEngine {
        TextEngine::new()
    }

    const SURFACE: Size = Size::new(400, 240);
    const ANCHOR: Rect = Rect::new(100, 100, 80, 30);

    /// Nothing appears before the delay, and something does after it.
    #[test]
    fn the_bubble_waits_for_the_pointer_to_rest() {
        let mut tooltip = Tooltip::new();
        assert_eq!(tooltip.phase, Phase::Idle);
        assert_eq!(tooltip.next_wake(), None, "an idle tree wakes for nothing");

        tooltip.hover_changed(true, 1_000);
        assert_eq!(tooltip.next_wake(), Some(1_000 + DELAY_MS));

        assert!(!tooltip.tick(1_000, Some("Lagre"), ANCHOR), "not yet");
        assert!(!tooltip.tick(1_000 + DELAY_MS - 1, Some("Lagre"), ANCHOR));
        assert!(tooltip.tick(1_000 + DELAY_MS, Some("Lagre"), ANCHOR), "now");
        assert_eq!(tooltip.phase, Phase::Shown);
        assert_eq!(tooltip.next_wake(), None, "a shown bubble wants no more");
    }

    /// Hovering something without a tooltip is not a wait, it is idle — or the
    /// tree would wake for every widget on the panel.
    #[test]
    fn hovering_something_without_a_tooltip_wakes_nothing() {
        let mut tooltip = Tooltip::new();
        tooltip.hover_changed(false, 1_000);
        assert_eq!(tooltip.phase, Phase::Idle);
        assert_eq!(tooltip.next_wake(), None);
        assert!(!tooltip.tick(9_999, Some("Lagre"), ANCHOR));
    }

    /// Moving on takes the bubble away and reports that it did, so the tree
    /// knows to repaint what it covered.
    #[test]
    fn moving_on_takes_the_bubble_away() {
        let mut tooltip = Tooltip::new();
        tooltip.hover_changed(true, 0);
        tooltip.tick(DELAY_MS, Some("Lagre"), ANCHOR);
        assert_eq!(tooltip.phase, Phase::Shown);

        assert!(tooltip.hover_changed(false, DELAY_MS), "it was showing");
        assert_eq!(tooltip.phase, Phase::Idle);
        assert!(tooltip.shown.is_none(), "and it let go of the text");
        assert!(
            !tooltip.hover_changed(false, DELAY_MS),
            "nothing was showing the second time"
        );
    }

    /// A press or a key means the person moved on, whatever the pointer is over.
    #[test]
    fn a_press_or_a_key_dismisses_it() {
        let mut tooltip = Tooltip::new();
        tooltip.hover_changed(true, 0);
        tooltip.tick(DELAY_MS, Some("Lagre"), ANCHOR);
        assert!(tooltip.dismiss(), "it was showing");
        assert_eq!(tooltip.phase, Phase::Idle);
        assert!(!tooltip.dismiss());

        // And dismissing mid-wait stops the wait rather than leaving a deadline
        // the tree would wake for.
        tooltip.hover_changed(true, 0);
        assert!(tooltip.next_wake().is_some());
        tooltip.dismiss();
        assert_eq!(tooltip.next_wake(), None);
    }

    /// Moving between two widgets that both have tooltips restarts the wait
    /// rather than showing the second one instantly.
    #[test]
    fn moving_between_two_tooltips_restarts_the_wait() {
        let mut tooltip = Tooltip::new();
        tooltip.hover_changed(true, 0);
        tooltip.tick(DELAY_MS, Some("Lagre"), ANCHOR);
        assert_eq!(tooltip.phase, Phase::Shown);

        tooltip.hover_changed(true, DELAY_MS);
        assert_eq!(
            tooltip.next_wake(),
            Some(DELAY_MS * 2),
            "the second wait starts from the move"
        );
        assert!(!tooltip.tick(DELAY_MS, Some("Avbryt"), ANCHOR), "not yet");
    }

    /// A node that loses its tooltip while the pointer rests on it shows
    /// nothing, rather than an empty bubble.
    #[test]
    fn a_tooltip_removed_mid_wait_shows_nothing() {
        let mut tooltip = Tooltip::new();
        tooltip.hover_changed(true, 0);
        assert!(!tooltip.tick(DELAY_MS, None, ANCHOR));
        assert_eq!(tooltip.phase, Phase::Idle);

        tooltip.hover_changed(true, 0);
        assert!(
            !tooltip.tick(DELAY_MS, Some(""), ANCHOR),
            "nor an empty one"
        );
    }

    /// A tooltip is drawn at the size it was told, so it scales with everything
    /// else.
    ///
    /// The tree has no idea what the display's scale factor is — an application
    /// multiplies its sizes once, at construction — and this was the last thing
    /// on the screen with no way to be told. A designer at 2x had a chrome at
    /// twice the size and a tooltip still at fourteen pixels.
    #[test]
    fn a_bubble_is_drawn_at_the_size_it_was_given() {
        let mut engine = engine();
        let mut tooltip = Tooltip::new();
        tooltip.hover_changed(true, 0);
        tooltip.tick(DELAY_MS, Some("Lagre"), ANCHOR);
        let small = tooltip.bounds(SURFACE, &mut engine).expect("a bubble");

        tooltip.set_size(28);
        let large = tooltip.bounds(SURFACE, &mut engine).expect("a bubble");
        assert!(
            large.width > small.width && large.height > small.height,
            "{large:?} is no larger than {small:?}"
        );

        tooltip.set_size(0);
        assert!(
            tooltip
                .bounds(SURFACE, &mut engine)
                .is_some_and(|r| r.height > 0),
            "a bubble with no text in it is worse than one at the wrong size"
        );
    }

    /// The bubble sits below its anchor, and flips above near the bottom edge —
    /// the same rule every popup follows.
    #[test]
    fn the_bubble_is_placed_below_and_flips_near_the_edge() {
        let mut engine = engine();
        let below = place(
            SURFACE,
            ANCHOR,
            "Lagre",
            TextStyle::built_in(14),
            &mut engine,
        );
        assert_eq!(below.y, ANCHOR.bottom() + GAP);
        assert!(below.width > 0 && below.height > 0);

        let low = Rect::new(100, 210, 80, 25);
        let above = place(SURFACE, low, "Lagre", TextStyle::built_in(14), &mut engine);
        assert_eq!(above.bottom(), low.y - GAP, "flipped above");

        // And it never leaves the surface, wherever the anchor is.
        for anchor in [
            Rect::new(-40, -40, 30, 20),
            Rect::new(380, 220, 30, 20),
            Rect::new(0, 0, 0, 0),
        ] {
            let rect = place(
                SURFACE,
                anchor,
                "Lagre lenge",
                TextStyle::built_in(14),
                &mut engine,
            );
            assert!(rect.x >= 0 && rect.y >= 0, "{anchor:?} gave {rect:?}");
            assert!(
                rect.right() <= SURFACE.width as i32 && rect.bottom() <= SURFACE.height as i32,
                "{anchor:?} gave {rect:?}"
            );
        }
    }

    /// The bubble is bigger than its text, so the padding is actually there.
    #[test]
    fn the_bubble_has_padding_round_its_text() {
        let mut engine = engine();
        let style = TextStyle::built_in(14);
        let extent = engine.measure(style, "Lagre");
        let bubble = place(SURFACE, ANCHOR, "Lagre", style, &mut engine);
        assert_eq!(bubble.width, extent.width as i32 + PADDING * 2);
        assert_eq!(bubble.height, extent.height as i32 + PADDING * 2);
    }

    /// A longer tooltip makes a wider bubble — it is measured, not guessed.
    #[test]
    fn a_longer_tooltip_makes_a_wider_bubble() {
        let mut engine = engine();
        let style = TextStyle::built_in(14);
        let short = place(SURFACE, ANCHOR, "Ja", style, &mut engine);
        let long = place(SURFACE, ANCHOR, "Lagre endringene", style, &mut engine);
        assert!(long.width > short.width);
    }
}
