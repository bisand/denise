//! How many screen pixels the canvas draws one form pixel as.
//!
//! # Why this is not the display's scale factor
//!
//! [`Scale`](crate::scale::Scale) is the display's, applies to the **chrome**,
//! and is not a choice anybody makes. This is the canvas's, applies to the
//! **form**, and is entirely a choice — it exists because a form authored for an
//! 800x480 panel is a postage stamp on a dense desktop display, and because the
//! same form is unreadable on a laptop when it was drawn for a 1920x1080 one.
//!
//! They are deliberately separate. #153 gave the chrome the scale factor and
//! left the canvas at 1:1, because scaling the stage would have put a coordinate
//! conversion into the same change as a multiplication, and the two look alike
//! and are not. This is that conversion, on its own, with the rule it has to
//! obey stated once:
//!
//! > **The numbers never change.** `width 800` in the inspector means 800 at
//! > every zoom level, a drag of one form pixel writes one form pixel, and the
//! > grid snaps to the grid the file records. Zoom changes what a form pixel
//! > *looks* like and nothing else.
//!
//! # Which way round
//!
//! [`Zoom::on_screen`] goes **form to screen** — the direction a rectangle travels on
//! its way into the tree, since `Form::build_scaled` puts the whole form subtree
//! in screen units. [`Zoom::in_form`] goes back, and is the direction every number
//! bound for the file or the inspector travels.
//!
//! Rectangles convert **by their edges**, for the reason
//! [`Rect::scaled`] gives: two widgets that touch in the file still touch at
//! 150%, where scaling two extents apart would open a seam between them.
//!
//! # The round trip is not free below 100%
//!
//! At 200% a form pixel is two screen pixels and `in_form(on_screen(n)) == n` for
//! every `n`. At 50% it is half of one, and `in_form(on_screen(11))` is 12: the
//! information is
//! genuinely gone. That is why a drag keeps its **form** rectangle as the thing
//! it is really editing and hands the tree a copy, rather than reading back what
//! it just wrote — a node must never drift by a pixel because somebody zoomed
//! out and nudged something else.

use denise::{Rect, Size};

/// The canvas's magnification, as a percentage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Zoom {
    /// Screen pixels per hundred form pixels.
    percent: u16,
    /// Whether it follows the viewport rather than sitting on a step.
    ///
    /// Kept alongside the percentage rather than instead of it, so that every
    /// conversion has a factor to use without knowing how big the canvas is —
    /// `fitted` is what recomputes it when the window changes.
    fit: bool,
}

impl Default for Zoom {
    fn default() -> Self {
        Self::ACTUAL
    }
}

impl Zoom {
    /// One screen pixel per form pixel, which is what #153 left the canvas at
    /// and what every snapshot still uses.
    pub const ACTUAL: Self = Self {
        percent: 100,
        fit: false,
    };

    /// The longest thing [`Zoom::label`] can return.
    ///
    /// The toolbar sizes a button from the text it is built with and does not
    /// resize it afterwards, so the control is built with this and relabelled on
    /// the first frame. `fit (100%)` is the widest: every step is shorter, and
    /// no fit is wider than a hundred per cent.
    pub const WIDEST_LABEL: &'static str = "fit (100%)";

    /// The steps `+` and `-` walk, and the ones the control offers.
    ///
    /// Doubling, with 100 on it, so that every step but the ends is a whole
    /// number of screen pixels per form pixel in one direction or the other —
    /// which is the difference between a crisp form and a resampled one.
    pub const STEPS: [u16; 6] = [25, 50, 100, 200, 400, 800];

    /// The narrowest and widest the fit is allowed to be.
    const FLOOR: u16 = 10;
    const CEILING: u16 = 800;

    /// A zoom at a step, clamped to one that can be drawn.
    pub fn at(percent: u16) -> Self {
        Self {
            percent: percent.clamp(Self::FLOOR, Self::CEILING),
            fit: false,
        }
    }

    /// A zoom that follows the viewport, starting from `percent`.
    pub fn fitted(percent: u16) -> Self {
        Self {
            percent: percent.clamp(Self::FLOOR, Self::CEILING),
            fit: true,
        }
    }

    /// Whether it is following the viewport.
    #[inline]
    pub const fn is_fit(self) -> bool {
        self.fit
    }

    /// Whether one form pixel is one screen pixel, and the conversions are
    /// therefore identity.
    #[inline]
    pub const fn is_actual(self) -> bool {
        self.percent == 100
    }

    /// The percentage itself.
    #[inline]
    pub const fn percent(self) -> u16 {
        self.percent
    }

    /// What it says on the control.
    pub fn label(self) -> String {
        if self.fit {
            format!("fit ({}%)", self.percent)
        } else {
            format!("{}%", self.percent)
        }
    }

    /// The factor `Form::build_scaled` and `Theme::scaled` take.
    #[inline]
    pub fn factor(self) -> f32 {
        f32::from(self.percent) / 100.0
    }

    /// The largest step at or below this one, for `-`.
    pub fn narrower(self) -> Self {
        let next = Self::STEPS
            .iter()
            .rev()
            .find(|step| **step < self.percent)
            .copied()
            .unwrap_or(Self::STEPS[0]);
        Self::at(next)
    }

    /// The smallest step above this one, for `+`.
    pub fn wider(self) -> Self {
        let next = Self::STEPS
            .iter()
            .find(|step| **step > self.percent)
            .copied()
            .unwrap_or(Self::CEILING);
        Self::at(next)
    }

    /// The zoom at which `design` fits inside `view` with `margin` to spare.
    ///
    /// Never above 100%: "fit" means *make it all visible*, and blowing a small
    /// form up to fill a large window is a different thing that the steps
    /// already offer. A viewport too small to show anything falls back to the
    /// floor rather than to zero.
    pub fn to_fit(design: Size, view: Size, margin: i32) -> Self {
        let room = |extent: u32, taken: i32| i32::try_from(extent).unwrap_or(i32::MAX) - taken * 2;
        let (across, down) = (room(view.width, margin), room(view.height, margin));
        if design.width == 0 || design.height == 0 || across <= 0 || down <= 0 {
            return Self::fitted(100);
        }
        let by_width = across as i64 * 100 / i64::from(design.width);
        let by_height = down as i64 * 100 / i64::from(design.height);
        let percent = by_width.min(by_height).clamp(0, 100) as u16;
        Self::fitted(percent.max(Self::FLOOR))
    }

    /// A form rectangle as the tree holds it — **by its edges**.
    #[inline]
    pub fn on_screen(self, form: Rect) -> Rect {
        if self.is_actual() {
            return form;
        }
        form.scaled(self.factor())
    }

    /// A tree rectangle as the file writes it — **by its edges**.
    #[inline]
    pub fn in_form(self, screen: Rect) -> Rect {
        if self.is_actual() {
            return screen;
        }
        screen.scaled(1.0 / self.factor())
    }

    /// A form length on screen.
    #[inline]
    pub fn on_screen_n(self, form: i32) -> i32 {
        if self.is_actual() {
            return form;
        }
        round(form as f32 * self.factor())
    }

    /// A screen length in form pixels.
    #[inline]
    pub fn in_form_n(self, screen: i32) -> i32 {
        if self.is_actual() {
            return screen;
        }
        round(screen as f32 / self.factor())
    }

    /// A form size on screen, as the stage has to be.
    pub fn on_screen_size(self, form: Size) -> Size {
        if self.is_actual() {
            return form;
        }
        let extent = |v: u32| {
            let scaled = round(v as f32 * self.factor());
            u32::try_from(scaled.max(0)).unwrap_or(0)
        };
        Size::new(extent(form.width), extent(form.height))
    }
}

/// Rounds half away from zero, matching [`Rect::scaled`].
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
    fn actual_size_is_identity_in_both_directions() {
        let one = Zoom::ACTUAL;
        let rect = Rect::new(7, 11, 83, 29);
        assert_eq!(one.on_screen(rect), rect);
        assert_eq!(one.in_form(rect), rect);
        assert_eq!(one.on_screen_n(13), 13);
        assert_eq!(one.in_form_n(13), 13);
    }

    /// Magnifying and coming back is exact at or above 100%.
    ///
    /// Which is what lets a drag at 200% land on the number it aimed at. Below
    /// it the information is genuinely gone — see this module's header, and
    /// `Designer::dragged_to`, which is why that does not matter.
    #[test]
    fn a_magnified_rectangle_comes_back_the_same() {
        for percent in [100, 200, 400, 800] {
            let zoom = Zoom::at(percent);
            for rect in [
                Rect::new(0, 0, 1, 1),
                Rect::new(4, 4, 80, 20),
                Rect::new(137, 41, 3, 999),
            ] {
                assert_eq!(
                    zoom.in_form(zoom.on_screen(rect)),
                    rect,
                    "{rect:?} did not survive {percent}%"
                );
            }
        }
    }

    /// Two widgets that touch in the file still touch on screen.
    ///
    /// The reason rectangles convert by their edges. At 150% a naive
    /// width-times-factor opens a one-pixel seam between panels that were drawn
    /// against each other, and it opens it at some coordinates and not others,
    /// which is the worst way for it to be wrong.
    #[test]
    fn a_seam_does_not_open_at_an_awkward_magnification() {
        let zoom = Zoom::at(150);
        let left = zoom.on_screen(Rect::new(0, 0, 7, 40));
        let right = zoom.on_screen(Rect::new(7, 0, 9, 40));
        assert_eq!(left.right(), right.x, "a seam opened between them");
    }

    #[test]
    fn the_steps_walk_both_ways_and_stop_at_the_ends() {
        assert_eq!(Zoom::at(100).wider().percent(), 200);
        assert_eq!(Zoom::at(100).narrower().percent(), 50);
        assert_eq!(Zoom::at(800).wider().percent(), 800, "stops at the widest");
        assert_eq!(
            Zoom::at(25).narrower().percent(),
            25,
            "stops at the narrowest"
        );
        // From somewhere between two steps, to the next one either way.
        assert_eq!(Zoom::at(150).wider().percent(), 200);
        assert_eq!(Zoom::at(150).narrower().percent(), 100);
    }

    /// Fitting shows the whole form, and never blows a small one up.
    #[test]
    fn a_fit_shows_all_of_it() {
        let design = Size::new(800, 480);

        // Half the width available, so half the size — and the narrower axis
        // wins, because both have to fit.
        let fit = Zoom::to_fit(design, Size::new(400, 1000), 0);
        assert_eq!(fit.percent(), 50);
        assert!(fit.is_fit());

        let squat = Zoom::to_fit(design, Size::new(1600, 240), 0);
        assert_eq!(squat.percent(), 50, "the short axis decides");

        // Room to spare is not a reason to magnify: fit means *all of it*.
        assert_eq!(
            Zoom::to_fit(design, Size::new(4000, 4000), 0).percent(),
            100
        );

        // The margin comes off both sides.
        assert_eq!(Zoom::to_fit(design, Size::new(816, 1000), 8).percent(), 100);
        assert!(Zoom::to_fit(design, Size::new(800, 1000), 8).percent() < 100);
    }

    /// A viewport with no room in it still produces something drawable.
    #[test]
    fn a_form_or_a_window_of_nothing_does_not_produce_a_zoom_of_nothing() {
        for (design, view) in [
            (Size::new(0, 0), Size::new(800, 600)),
            (Size::new(800, 480), Size::new(0, 0)),
            (Size::new(800, 480), Size::new(4, 4)),
        ] {
            let fit = Zoom::to_fit(design, view, 8);
            assert!(fit.percent() > 0, "{design:?} in {view:?} gave nothing");
            assert!(fit.factor() > 0.0);
        }
    }

    #[test]
    fn the_control_says_which_it_is() {
        assert_eq!(Zoom::at(200).label(), "200%");
        assert_eq!(Zoom::fitted(71).label(), "fit (71%)");
    }

    /// Nothing the control can say is wider than the button built for it.
    #[test]
    fn the_button_is_built_wide_enough_for_anything_it_will_say() {
        let widest = Zoom::WIDEST_LABEL.chars().count();
        for percent in Zoom::STEPS {
            assert!(Zoom::at(percent).label().chars().count() <= widest);
            assert!(Zoom::fitted(percent).label().chars().count() <= widest);
        }
        assert!(Zoom::fitted(100).label().chars().count() <= widest);
    }
}
