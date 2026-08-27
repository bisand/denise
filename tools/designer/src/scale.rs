//! The display's scale factor, and the three ways this designer applies it.
//!
//! `docs/design.md` settles who multiplies: **the application scales, once, at
//! construction**. It already knows the factor and already computes every
//! rectangle, so it is the one place that can multiply everything consistently.
//! The pattern there is three calls — `Theme::scaled` for the widgets'
//! furniture, [`Rect::scaled`] for every layout rectangle, and a scaled text
//! size wherever one is named — and this is those three, named, so the chrome
//! can be written in logical units and say so.
//!
//! The designer went without for long enough to be the example the rule was
//! written against: it took the factor from `run_with` and dropped it, so on a
//! 2x display every pane, every row and every label came out at half the size
//! it was drawn at. Correct on a Pi, which is why nothing caught it.
//!
//! # Logical above, physical below
//!
//! Every constant in this crate is **logical**, and stays logical: it is written
//! once, at the size a person reading the source would expect, and multiplied on
//! the way into the tree. What comes back *out* of the tree — `Ui::bounds`, a
//! pointer position, a scroll offset — is physical, so a constant compared
//! against one of those needs [`Scale::n`] first. That asymmetry is the whole of
//! what there is to get wrong here, and the test that catches it is
//! `the_chrome_at_twice_the_scale_is_the_same_layout_doubled`: it builds the
//! designer twice and requires every node of the chrome, and every text size in
//! it, to be exactly double at 2x. A missed multiplication is invisible at 1x,
//! which is where the rest of the suite lives.
//!
//! # The canvas has its own multiplication
//!
//! This one is the chrome's. A form is authored in the panel's own device
//! pixels, and the display's density is not a reason to change them — so the
//! canvas does not follow this factor at all. What magnifies it is
//! [`Zoom`](crate::zoom::Zoom), which is a *choice* rather than a property of
//! the screen, and which converts in both directions so that the numbers in the
//! file never move. The two are deliberately separate: they both look like
//! multiplication and only one of them is.

use denise::Rect;

use crate::text::Text;

/// A display scale factor, applied by the three calls below.
///
/// Cheap to copy and passed by value, because it is one `f32` and threading a
/// reference to it through the chrome would be worse than the multiplication.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale(f32);

impl Default for Scale {
    fn default() -> Self {
        Self::ONE
    }
}

impl Scale {
    /// One physical pixel per logical one: a panel, and every snapshot.
    pub const ONE: Self = Self(1.0);

    /// The factor a window reported, with nonsense refused.
    ///
    /// A scale factor arrives from the display server, and a zero, a negative or
    /// a NaN would produce a window with no chrome in it and no clue as to why.
    /// Clamped rather than rejected: the upper bound is well past any real
    /// display, and is there so a wrong reading cannot ask for a rectangle that
    /// overflows on the way in.
    pub fn new(factor: f32) -> Self {
        if factor.is_finite() && factor > 0.0 {
            Self(factor.clamp(0.25, 8.0))
        } else {
            Self::ONE
        }
    }

    /// The factor itself, for `Theme::scaled` and the like.
    #[inline]
    pub const fn factor(self) -> f32 {
        self.0
    }

    /// A layout rectangle in physical pixels.
    ///
    /// By its **edges**, which is why it delegates rather than multiplying two
    /// extents: panes that touch in the logical layout still touch at 1.5x.
    #[inline]
    pub fn r(self, rect: Rect) -> Rect {
        rect.scaled(self.0)
    }

    /// A single length in physical pixels.
    ///
    /// For the lengths that never become a rectangle edge — a hit-test radius, a
    /// drag threshold, a row height divided into a pointer offset. Prefer
    /// [`Scale::r`] wherever there is a rectangle to scale, because a pair of
    /// lengths rounded apart is the seam that method exists to close.
    #[inline]
    pub fn n(self, length: i32) -> i32 {
        round(length as f32 * self.0)
    }

    /// A step of the designer's type scale, in physical pixels.
    ///
    /// The way text sizes are named here: [`Text`] has four steps and there is
    /// no fifth to write. See its module for why the numbers are those numbers.
    #[inline]
    pub fn text(self, text: Text) -> u16 {
        self.px(text.px())
    }

    /// A text size in physical pixels, never rounded away to nothing.
    ///
    /// Prefer [`Scale::text`]. This is for the few sizes that are not the
    /// designer's own — a size the *form* asked for, at the canvas's zoom.
    #[inline]
    pub fn px(self, size_px: u16) -> u16 {
        let scaled = round(f32::from(size_px) * self.0);
        scaled.clamp(1, i32::from(u16::MAX)) as u16
    }

    /// Back the other way: a physical extent as the logical one to remember.
    ///
    /// The window size in [`Settings`](crate::settings::Settings) is logical —
    /// it is handed back to `WindowConfig`, which is logical — while the resize
    /// event that carries it is physical. Without this the remembered size grows
    /// by the scale factor on every run.
    #[inline]
    pub fn logical(self, physical: u32) -> u32 {
        round(physical as f32 / self.0).max(0) as u32
    }
}

/// Rounds half away from zero, matching [`Rect::scaled`].
///
/// Spelled out rather than `f32::round` for the same reason the geometry does
/// it: this crate is `std`, but the rounding has to agree with the `no_std` one
/// or a rectangle and the length beside it disagree by a pixel at 1.5x.
#[inline]
fn round(v: f32) -> i32 {
    if v >= 0.0 {
        (v + 0.5) as i32
    } else {
        -((0.5 - v) as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_moves_at_one() {
        let one = Scale::ONE;
        assert_eq!(one.r(Rect::new(8, 30, 240, 44)), Rect::new(8, 30, 240, 44));
        assert_eq!(one.n(13), 13);
        assert_eq!(one.px(11), 11);
        assert_eq!(one.logical(1280), 1280);
    }

    #[test]
    fn a_display_scale_that_makes_no_sense_is_no_scale_at_all() {
        for absurd in [0.0, -2.0, f32::NAN, f32::INFINITY] {
            assert_eq!(Scale::new(absurd), Scale::ONE, "{absurd} was taken");
        }
        assert_eq!(Scale::new(64.0).factor(), 8.0, "clamped, not taken");
    }

    /// Panes that touch at 1x touch at 1.5x, which is why `r` scales edges.
    ///
    /// The naive version multiplies width and rounds, and two panels that shared
    /// an edge round apart — a one-pixel seam of whatever is behind them, down
    /// the middle of a window, at exactly the scale factor Windows uses most.
    #[test]
    fn a_fractional_scale_does_not_open_a_seam() {
        let scale = Scale::new(1.5);
        let left = scale.r(Rect::new(0, 0, 7, 40));
        let right = scale.r(Rect::new(7, 0, 7, 40));
        assert_eq!(left.right(), right.x, "a seam opened between them");
    }

    /// A text size never rounds away to nothing.
    #[test]
    fn small_text_on_a_coarse_display_is_still_text() {
        assert_eq!(Scale::new(0.25).px(2), 1);
        assert_eq!(Scale::new(2.0).px(11), 22);
    }

    /// The window a run remembers is the window the next run opens.
    #[test]
    fn a_physical_surface_is_remembered_as_the_logical_window() {
        let two = Scale::new(2.0);
        assert_eq!(two.logical(2560), 1280);
        assert_eq!(two.logical(two.n(800) as u32), 800, "there and back");
    }
}
