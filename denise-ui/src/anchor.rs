//! What a node does when its parent is not the size it was designed against.
//!
//! Denise has no layout engine and is not getting one: a node is an explicit
//! rectangle relative to its parent, which is what a fixed-resolution panel
//! wants. But a form designed at 1024×600 that only works at 1024×600 is a form
//! that cannot go in a resizable window, cannot be turned to portrait, and cannot
//! shrink to stay above an on-screen keyboard — and all three of those are things
//! this toolkit already has to do.
//!
//! So there are two placement rules, and they are the two Delphi and the WinForms
//! designer had. Neither is a solver. Both are one derived rectangle per child,
//! computed in the pass [`Ui::reflow`] already runs over the tree.
//!
//! [`Ui::reflow`]: crate::Ui
//!
//! # Anchors
//!
//! [`Anchors`] names the parent edges a node keeps its distance from.
//!
//! - Anchored to **one** edge of an axis, the node keeps its distance from that
//!   edge and its own size. Anchored left, it does not move when the parent grows
//!   on the right — which is [the default](Anchors::TOP_LEFT), and is exactly what
//!   every tree written before this existed already did.
//! - Anchored to **both** edges, the node keeps both distances, so it **stretches**.
//! - Anchored to **neither**, its two gaps grow equally, so a node centred in its
//!   parent stays centred.
//!
//! # Docking
//!
//! [`Dock`] gives a node an entire edge of what is left of its parent, across the
//! full width or height of it. Docked children are placed **in paint order**, each
//! taking its edge from what the ones before it left — so two `Dock::Top`
//! children are two stacked bars, and a [`Dock::Fill`] takes whatever remains.
//! [`set_z`](crate::Ui::set_z) reorders docking as it reorders painting.
//!
//! Only the node's own extent along the docking axis is used: a `Dock::Top` node
//! keeps its `height` and is given the full width.
//!
//! # What stays true
//!
//! **A node's stored `layout` is never rewritten.** Both rules *derive* a
//! rectangle, exactly as the vertical stack and the scroll offset already do, and
//! for the same reason: paint, damage, clipping and hit testing all read what one
//! pass wrote, so they cannot disagree. It is also what lets a form file keep one
//! rectangle per node — a designer moving a button still produces a one-line diff,
//! whatever the anchors do afterwards.

use denise::{Rect, Size};

/// Which of its parent's edges a node keeps its distance from.
///
/// See the [module documentation](self) for what each combination means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Anchors {
    /// Keep the distance from the parent's left edge.
    pub left: bool,
    /// Keep the distance from the parent's top edge.
    pub top: bool,
    /// Keep the distance from the parent's right edge.
    pub right: bool,
    /// Keep the distance from the parent's bottom edge.
    pub bottom: bool,
}

impl Anchors {
    /// Left and top: the node keeps its position and its size.
    ///
    /// **The default**, and deliberately so — it is what the tree did before
    /// anchoring existed, so every tree and every form written until now behaves
    /// exactly as it did.
    pub const TOP_LEFT: Self = Self {
        left: true,
        top: true,
        right: false,
        bottom: false,
    };

    /// All four edges: the node stretches with its parent on both axes.
    pub const STRETCH: Self = Self {
        left: true,
        top: true,
        right: true,
        bottom: true,
    };

    /// No edges: the node keeps its size, and both its gaps grow equally, so
    /// something centred stays centred.
    pub const CENTER: Self = Self {
        left: false,
        top: false,
        right: false,
        bottom: false,
    };

    /// Anchors from the four edges.
    pub const fn new(left: bool, top: bool, right: bool, bottom: bool) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// Returns `true` if this node's rectangle does not depend on its parent's
    /// size — the case the reflow can skip entirely.
    pub const fn is_fixed(self) -> bool {
        self.left && self.top && !self.right && !self.bottom
    }
}

impl Default for Anchors {
    fn default() -> Self {
        Self::TOP_LEFT
    }
}

/// An edge of its parent that a node takes entirely.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dock {
    /// The full width of what is left, at the top, keeping the node's height.
    Top,
    /// The full width of what is left, at the bottom, keeping the node's height.
    Bottom,
    /// The full height of what is left, at the left, keeping the node's width.
    Left,
    /// The full height of what is left, at the right, keeping the node's width.
    Right,
    /// Everything that is left.
    Fill,
}

/// One axis of the anchoring rule.
///
/// `position` and `extent` are the node's; `growth` is how much longer the parent
/// is than the node was designed against, which may be negative.
const fn anchored_axis(
    position: i32,
    extent: i32,
    growth: i32,
    low: bool,
    high: bool,
) -> (i32, i32) {
    match (low, high) {
        // Both edges held, so the distance to each is kept and the node covers
        // the difference. Never narrower than nothing: a parent that shrank past
        // this node's margins clips it rather than inverting it.
        (true, true) => {
            // `Ord::max` is not const yet.
            let stretched = extent + growth;
            (position, if stretched < 0 { 0 } else { stretched })
        }
        // The near edge only: the node is where it always was.
        (true, false) => (position, extent),
        // The far edge only: the node moves by the whole difference and keeps
        // its size.
        (false, true) => (position + growth, extent),
        // Neither: both gaps take half, so something centred stays centred.
        (false, false) => (position + growth / 2, extent),
    }
}

/// A node's rectangle, given the parent size it was designed against and the one
/// the parent actually has.
///
/// Relative to the parent's origin, like the `layout` it derives from.
pub fn anchored(layout: Rect, base: Size, current: Size, anchors: Anchors) -> Rect {
    let dw = current.width as i32 - base.width as i32;
    let dh = current.height as i32 - base.height as i32;
    let (x, width) = anchored_axis(layout.x, layout.width, dw, anchors.left, anchors.right);
    let (y, height) = anchored_axis(layout.y, layout.height, dh, anchors.top, anchors.bottom);
    Rect::new(x, y, width, height)
}

/// A docked node's rectangle, and what is left of `remaining` afterwards.
///
/// `remaining` starts as the parent's whole content box and shrinks as each
/// docked child in paint order takes its edge.
pub fn docked(layout: Rect, remaining: Rect, dock: Dock) -> (Rect, Rect) {
    // A node cannot take more than there is, and cannot take less than nothing.
    let want_h = layout.height.clamp(0, remaining.height.max(0));
    let want_w = layout.width.clamp(0, remaining.width.max(0));

    match dock {
        Dock::Top => (
            Rect::new(remaining.x, remaining.y, remaining.width, want_h),
            Rect::new(
                remaining.x,
                remaining.y + want_h,
                remaining.width,
                remaining.height - want_h,
            ),
        ),
        Dock::Bottom => (
            Rect::new(
                remaining.x,
                remaining.y + remaining.height - want_h,
                remaining.width,
                want_h,
            ),
            Rect::new(
                remaining.x,
                remaining.y,
                remaining.width,
                remaining.height - want_h,
            ),
        ),
        Dock::Left => (
            Rect::new(remaining.x, remaining.y, want_w, remaining.height),
            Rect::new(
                remaining.x + want_w,
                remaining.y,
                remaining.width - want_w,
                remaining.height,
            ),
        ),
        Dock::Right => (
            Rect::new(
                remaining.x + remaining.width - want_w,
                remaining.y,
                want_w,
                remaining.height,
            ),
            Rect::new(
                remaining.x,
                remaining.y,
                remaining.width - want_w,
                remaining.height,
            ),
        ),
        Dock::Fill => (
            remaining,
            Rect::new(remaining.x, remaining.y, 0, remaining.height),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: Size = Size::new(100, 100);

    fn grown(layout: Rect, anchors: Anchors, to: Size) -> Rect {
        anchored(layout, BASE, to, anchors)
    }

    #[test]
    fn the_default_is_what_the_tree_did_before_anchoring_existed() {
        let layout = Rect::new(10, 20, 30, 40);
        for size in [Size::new(100, 100), Size::new(200, 300), Size::new(40, 40)] {
            assert_eq!(
                grown(layout, Anchors::TOP_LEFT, size),
                layout,
                "a top-left node must not move or resize, whatever the parent does"
            );
        }
        assert!(Anchors::TOP_LEFT.is_fixed());
        assert_eq!(Anchors::default(), Anchors::TOP_LEFT);
    }

    #[test]
    fn a_right_anchored_node_follows_the_right_edge_and_keeps_its_size() {
        let layout = Rect::new(60, 10, 30, 20);
        let anchors = Anchors::new(false, true, true, false);
        // 40 wider: the node moves 40 right, so its 10px gap to the right edge
        // is still 10.
        let grown = grown(layout, anchors, Size::new(140, 100));
        assert_eq!(grown, Rect::new(100, 10, 30, 20));
        assert_eq!(
            140 - (grown.x + grown.width),
            100 - (layout.x + layout.width)
        );
    }

    #[test]
    fn a_node_anchored_to_both_edges_stretches() {
        let layout = Rect::new(10, 10, 80, 20);
        let anchors = Anchors::new(true, true, true, false);
        let grown = grown(layout, anchors, Size::new(150, 100));
        assert_eq!(grown, Rect::new(10, 10, 130, 20));
        // Both margins survive, which is the whole claim.
        assert_eq!(grown.x, layout.x);
        assert_eq!(
            150 - (grown.x + grown.width),
            100 - (layout.x + layout.width)
        );
    }

    #[test]
    fn a_stretched_node_stops_at_nothing_rather_than_inverting() {
        let layout = Rect::new(10, 10, 80, 20);
        let squeezed = grown(layout, Anchors::STRETCH, Size::new(20, 20));
        assert_eq!(squeezed.width, 0, "a negative width is not a rectangle");
        assert_eq!(squeezed.height, 0);
    }

    #[test]
    fn an_unanchored_axis_keeps_a_centred_node_centred() {
        // 20 wide in a 100 parent, at 40: ten on each side of centre.
        let layout = Rect::new(40, 40, 20, 20);
        let grown = grown(layout, Anchors::CENTER, Size::new(200, 200));
        assert_eq!(grown, Rect::new(90, 90, 20, 20));
        assert_eq!(grown.x, (200 - grown.width) / 2);
    }

    #[test]
    fn docking_takes_an_edge_and_leaves_the_rest() {
        let all = Rect::new(0, 0, 200, 100);

        let (top, rest) = docked(Rect::new(0, 0, 999, 20), all, Dock::Top);
        assert_eq!(top, Rect::new(0, 0, 200, 20), "full width, its own height");
        assert_eq!(rest, Rect::new(0, 20, 200, 80));

        let (left, rest) = docked(Rect::new(0, 0, 50, 999), rest, Dock::Left);
        assert_eq!(left, Rect::new(0, 20, 50, 80), "full remaining height");
        assert_eq!(rest, Rect::new(50, 20, 150, 80));

        let (bottom, rest) = docked(Rect::new(0, 0, 0, 30), rest, Dock::Bottom);
        assert_eq!(bottom, Rect::new(50, 70, 150, 30));
        assert_eq!(rest, Rect::new(50, 20, 150, 50));

        let (right, rest) = docked(Rect::new(0, 0, 25, 0), rest, Dock::Right);
        assert_eq!(right, Rect::new(175, 20, 25, 50));
        assert_eq!(rest, Rect::new(50, 20, 125, 50));

        let (fill, rest) = docked(Rect::new(9, 9, 9, 9), rest, Dock::Fill);
        assert_eq!(
            fill,
            Rect::new(50, 20, 125, 50),
            "fill ignores its own size"
        );
        assert_eq!(rest.width, 0, "nothing is left after a fill");
    }

    #[test]
    fn two_bars_docked_to_the_same_edge_stack_in_order() {
        let all = Rect::new(0, 0, 100, 100);
        let (first, rest) = docked(Rect::new(0, 0, 0, 10), all, Dock::Top);
        let (second, rest) = docked(Rect::new(0, 0, 0, 10), rest, Dock::Top);
        assert_eq!(first, Rect::new(0, 0, 100, 10));
        assert_eq!(second, Rect::new(0, 10, 100, 10));
        assert_eq!(rest, Rect::new(0, 20, 100, 80));
    }

    #[test]
    fn a_dock_cannot_take_more_room_than_is_left() {
        let small = Rect::new(0, 0, 40, 30);
        let (taken, rest) = docked(Rect::new(0, 0, 0, 500), small, Dock::Top);
        assert_eq!(taken, small);
        assert_eq!(rest.height, 0);
    }
}
