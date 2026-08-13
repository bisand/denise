//! Placing a rectangle beside another one, and flipping when it will not fit.
//!
//! The geometry every popup needs and none should implement: a dropdown opens
//! below its button, a tooltip sits above its target, and both go to the other
//! side when the surface runs out — a menu near the bottom edge that opened
//! downwards anyway would be a menu nobody can read.
//!
//! Pure functions, so the flipping rules are testable without a tree. The
//! mechanism that uses them — a scene with light dismiss — is
//! [`Ui::push_popup`](crate::Ui::push_popup); a tooltip needs no mechanism at
//! all, just a non-interactive node placed with [`anchored`] at a high z.

use denise::{Rect, Size};

/// Which side of its anchor a popup prefers.
///
/// A preference, not a promise: when the surface has no room on that side and
/// the opposite side fits, the popup flips. See [`anchored`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    /// Over the anchor, bottom edge against its top.
    Above,
    /// Under the anchor, top edge against its bottom. What a dropdown wants.
    Below,
    /// To the left of the anchor.
    Before,
    /// To the right of the anchor. What a submenu wants.
    After,
}

impl Side {
    /// The other side of the same axis.
    #[inline]
    pub const fn opposite(self) -> Side {
        match self {
            Side::Above => Side::Below,
            Side::Below => Side::Above,
            Side::Before => Side::After,
            Side::After => Side::Before,
        }
    }
}

/// Where a rectangle of `size` sits against `anchor`, on `side`, with `gap`
/// pixels between them — before any fitting.
const fn beside(anchor: Rect, size: Size, side: Side, gap: i32) -> Rect {
    let (w, h) = (size.width as i32, size.height as i32);
    match side {
        Side::Above => Rect::new(anchor.x, anchor.y - gap - h, w, h),
        Side::Below => Rect::new(anchor.x, anchor.bottom() + gap, w, h),
        Side::Before => Rect::new(anchor.x - gap - w, anchor.y, w, h),
        Side::After => Rect::new(anchor.right() + gap, anchor.y, w, h),
    }
}

/// Whether `rect` lies entirely inside a surface of `surface` size.
const fn fits(surface: Size, rect: Rect) -> bool {
    rect.x >= 0
        && rect.y >= 0
        && rect.right() <= surface.width as i32
        && rect.bottom() <= surface.height as i32
}

/// Places a rectangle of `size` beside `anchor`, preferring `side`.
///
/// The rules, in order — each exists because skipping it produces a popup
/// somebody has actually seen misbehave:
///
/// 1. **The preferred side, if it fits.** The caller's choice is respected
///    whenever the surface allows it, so a layout does not reflow just because
///    the surface got bigger.
/// 2. **The opposite side, if the preferred one does not fit and the opposite
///    does.** The dropdown near the bottom edge opens upwards. Flipping only on
///    the anchor's own axis: a menu asked to open below never appears to the
///    left of its button.
/// 3. **Clamped into the surface otherwise.** When neither side has room —
///    a popup taller than the screen above *and* below a mid-screen anchor —
///    the preferred side is kept and the rectangle is slid inside the surface.
///    It may then cover its anchor, which is the least-bad answer: every part
///    of it is at least reachable.
///
/// Along the cross axis the popup is aligned to the anchor's leading edge —
/// the dropdown convention — and slid inside the surface when that would
/// overhang. A popup larger than the whole surface pins to the top-left, so
/// its origin is always visible.
pub fn anchored(surface: Size, anchor: Rect, size: Size, side: Side, gap: i32) -> Rect {
    let preferred = beside(anchor, size, side, gap);
    let flipped = beside(anchor, size, side.opposite(), gap);
    let mut rect = if fits(surface, preferred) || !fits(surface, flipped) {
        preferred
    } else {
        flipped
    };
    // Slide inside the surface. `max` last, so a rectangle too big for the
    // surface keeps its origin visible rather than its far corner.
    rect.x = rect.x.min(surface.width as i32 - rect.width).max(0);
    rect.y = rect.y.min(surface.height as i32 - rect.height).max(0);
    rect
}

#[cfg(test)]
mod tests {
    use super::*;

    const SURFACE: Size = Size::new(800, 480);
    /// A button somewhere in the middle.
    const ANCHOR: Rect = Rect::new(300, 200, 120, 40);

    #[test]
    fn the_preferred_side_is_used_when_it_fits() {
        let below = anchored(SURFACE, ANCHOR, Size::new(160, 100), Side::Below, 4);
        assert_eq!(below.y, ANCHOR.bottom() + 4);
        assert_eq!(below.x, ANCHOR.x, "aligned to the anchor's leading edge");

        let above = anchored(SURFACE, ANCHOR, Size::new(160, 100), Side::Above, 4);
        assert_eq!(above.bottom(), ANCHOR.y - 4);

        let after = anchored(SURFACE, ANCHOR, Size::new(160, 100), Side::After, 4);
        assert_eq!(after.x, ANCHOR.right() + 4);
        assert_eq!(after.y, ANCHOR.y);

        let before = anchored(SURFACE, ANCHOR, Size::new(160, 100), Side::Before, 4);
        assert_eq!(before.right(), ANCHOR.x - 4);
    }

    /// The dropdown near the bottom edge: no room below, so it opens upwards.
    #[test]
    fn a_popup_with_no_room_on_its_side_flips_to_the_other() {
        let anchor = Rect::new(300, 420, 120, 40);
        let rect = anchored(SURFACE, anchor, Size::new(160, 100), Side::Below, 4);
        assert_eq!(rect.bottom(), anchor.y - 4, "flipped above");

        let anchor = Rect::new(300, 10, 120, 40);
        let rect = anchored(SURFACE, anchor, Size::new(160, 100), Side::Above, 4);
        assert_eq!(rect.y, anchor.bottom() + 4, "flipped below");

        let anchor = Rect::new(700, 200, 90, 40);
        let rect = anchored(SURFACE, anchor, Size::new(160, 100), Side::After, 4);
        assert_eq!(rect.right(), anchor.x - 4, "flipped before");
    }

    /// Flipping happens on the anchor's own axis only: a menu asked to open
    /// below never appears beside its button.
    #[test]
    fn flipping_never_changes_the_axis() {
        let anchor = Rect::new(300, 420, 120, 40);
        let rect = anchored(SURFACE, anchor, Size::new(160, 100), Side::Below, 4);
        assert_eq!(rect.x, anchor.x, "still vertically attached");
        assert!(rect.bottom() <= anchor.y, "above, not beside");
    }

    /// Neither side has room: keep the preferred side, slide inside. The popup
    /// may cover its anchor; every part of it is at least reachable.
    #[test]
    fn a_popup_too_tall_for_either_side_is_clamped_inside() {
        let rect = anchored(SURFACE, ANCHOR, Size::new(160, 460), Side::Below, 4);
        assert!(rect.y >= 0, "top inside");
        assert!(rect.bottom() <= 480, "bottom inside");
    }

    /// The cross axis slides inside too: a dropdown on a button at the right
    /// edge must not overhang the surface.
    #[test]
    fn the_cross_axis_is_slid_inside_the_surface() {
        let anchor = Rect::new(700, 200, 90, 40);
        let rect = anchored(SURFACE, anchor, Size::new(240, 100), Side::Below, 4);
        assert_eq!(rect.right(), 800, "slid left to fit");
        assert_eq!(rect.y, anchor.bottom() + 4, "the main axis is untouched");
    }

    /// Bigger than the surface entirely: pinned at the origin, so the popup's
    /// own top-left — where its first content is — stays visible.
    #[test]
    fn a_popup_bigger_than_the_surface_pins_to_the_origin() {
        let rect = anchored(SURFACE, ANCHOR, Size::new(2000, 2000), Side::Below, 4);
        assert_eq!((rect.x, rect.y), (0, 0));
    }

    /// An anchor that is itself off the surface — scrolled away, or mid-layout —
    /// must still produce a rectangle inside the surface rather than a panic or
    /// a popup in space.
    #[test]
    fn an_absurd_anchor_still_lands_inside_the_surface() {
        for anchor in [
            Rect::new(-500, -500, 50, 20),
            Rect::new(5000, 5000, 50, 20),
            Rect::new(0, 0, 0, 0),
        ] {
            for side in [Side::Above, Side::Below, Side::Before, Side::After] {
                let rect = anchored(SURFACE, anchor, Size::new(100, 60), side, 4);
                assert!(fits(SURFACE, rect), "{anchor:?} {side:?} gave {rect:?}");
            }
        }
    }

    #[test]
    fn opposites_are_symmetric() {
        for side in [Side::Above, Side::Below, Side::Before, Side::After] {
            assert_eq!(side.opposite().opposite(), side);
        }
    }
}
