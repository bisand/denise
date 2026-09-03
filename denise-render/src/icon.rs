//! Shapes a widget can draw when the font has no glyph for them.
//!
//! # Why this exists
//!
//! A `⌫` on a Backspace key is a picture of an idea, and whether it can be
//! drawn at all currently depends on which font happens to be installed. The
//! answers differ more than one would guess: DejaVu has `⌫`, `⇥`, `⏎` and the
//! cursor triangles; a Mac's Arial has none of them and no triangle either; and
//! the face that ships with this crate has twenty-three non-ASCII glyphs of
//! which not one is either. A key that says "back" is legible everywhere and
//! looks like a compromise; a key that says `⌫` looks right and is a box on the
//! machine least able to spare one.
//!
//! An icon is drawn rather than looked up, so it is the same on every machine.
//!
//! # Filled polygons, and nothing else
//!
//! There is no path builder here and still is not one. An [`Icon`] is a short
//! list of closed polygons on a [`GRID`]-square box, scaled into whatever
//! rectangle it is asked for — which is enough for every shape a key or a
//! toolbar wants, and stops well short of a vector format this crate would have
//! to support forever.
//!
//! Strokes are absent for a reason rather than an oversight:
//! [`Canvas::draw_line`](crate::Canvas::draw_line) has no thickness and
//! deliberately does not, so a one-pixel outline on a 48-pixel key would be
//! invisible. A shape that reads as an outline is drawn as a filled polygon
//! with the middle knocked back out in [`Ink::Back`] — which is also how the
//! `×` inside `⌫` is made.
//!
//! # Coordinates
//!
//! Integers on a `0..=`[`GRID`] box, y downwards, scaled with integer
//! arithmetic. No floating point anywhere: this crate has neither `std` nor
//! `libm`, and the whole rasteriser is built on that.

pub use denise::icon::{GRID, Icon, Ink, MAX_SHAPES, Shape, fx_along};

use denise::{Color, Rect};

use crate::canvas::Canvas;

impl Canvas<'_> {
    /// Draws an icon into `rect`, scaled from its grid.
    ///
    /// `fore` is the content colour and `back` is whatever the icon is sitting
    /// on — a shape marked [`Ink::Back`] is drawn in it, which is how an outline
    /// or a cut-out is made.
    ///
    /// The icon is scaled to `rect` and **not** kept square: give it a square
    /// rectangle if you want it square. Anything beyond
    /// [`MAX_SHAPES`] is ignored rather than drawn wrong.
    pub fn draw_icon(&mut self, icon: &Icon, rect: Rect, fore: Color, back: Color) {
        crate::painter::Painter::draw_icon(self, icon, rect, fore, back);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestCanvas;

    const FORE: Color = Color::rgb(255, 255, 255);
    const BACK: Color = Color::rgb(0, 0, 0);

    /// The whole box, so a fill covers everything and a hole is unmistakable.
    static SQUARE: Icon = Icon::new(&[Shape::fore(&[(0, 0), (100, 0), (100, 100), (0, 100)])]);

    /// A square with the middle taken back out.
    static RING: Icon = Icon::new(&[
        Shape::fore(&[(0, 0), (100, 0), (100, 100), (0, 100)]),
        Shape::back(&[(30, 30), (70, 30), (70, 70), (30, 70)]),
    ]);

    #[test]
    fn an_icon_fills_the_rectangle_it_is_given() {
        let mut c = TestCanvas::new(40, 40);
        c.canvas()
            .draw_icon(&SQUARE, Rect::new(8, 8, 24, 24), FORE, BACK);

        assert_eq!(c.at(20, 20), FORE.to_argb8888(), "the middle is not filled");
        assert_eq!(c.at(2, 2), 0, "it painted outside its rectangle");
        assert_eq!(c.at(37, 37), 0, "it painted outside its rectangle");
    }

    /// The reason [`Ink::Back`] exists: the filler has no stroke and no
    /// even-odd rule, so an outline is a fill with the middle knocked out.
    #[test]
    fn a_back_shape_knocks_a_hole_in_the_one_before_it() {
        let mut c = TestCanvas::new(40, 40);
        c.canvas()
            .draw_icon(&RING, Rect::new(0, 0, 40, 40), FORE, BACK);

        assert_eq!(c.at(4, 20), FORE.to_argb8888(), "the ring is missing");
        assert_eq!(c.at(20, 20), BACK.to_argb8888(), "the hole was not punched");
    }

    /// Order is drawing order: a hole before its shape is painted over.
    #[test]
    fn a_hole_before_its_shape_is_covered_by_it() {
        static WRONG: Icon = Icon::new(&[
            Shape::back(&[(30, 30), (70, 30), (70, 70), (30, 70)]),
            Shape::fore(&[(0, 0), (100, 0), (100, 100), (0, 100)]),
        ]);
        let mut c = TestCanvas::new(40, 40);
        c.canvas()
            .draw_icon(&WRONG, Rect::new(0, 0, 40, 40), FORE, BACK);
        assert_eq!(
            c.at(20, 20),
            FORE.to_argb8888(),
            "shapes are drawn in order, so this hole should have been covered"
        );
    }

    /// The same icon at twice the size is the same icon, not a bigger sample of
    /// it. This is what a mask could not do and the reason these are polygons.
    #[test]
    fn an_icon_scales_rather_than_magnifies() {
        let mut small = TestCanvas::new(32, 32);
        small
            .canvas()
            .draw_icon(&RING, Rect::new(0, 0, 16, 16), FORE, BACK);
        let mut large = TestCanvas::new(32, 32);
        large
            .canvas()
            .draw_icon(&RING, Rect::new(0, 0, 32, 32), FORE, BACK);

        // Proportionally the same places: the ring at a tenth in, the hole in
        // the middle.
        for (c, side) in [(&small, 16), (&large, 32)] {
            let edge = side / 10;
            assert_eq!(
                c.at(edge, side / 2),
                FORE.to_argb8888(),
                "the ring is missing at {side}px"
            );
            assert_eq!(
                c.at(side / 2, side / 2),
                BACK.to_argb8888(),
                "the hole is missing at {side}px"
            );
        }
    }

    /// An empty rectangle draws nothing rather than dividing by zero.
    #[test]
    fn an_empty_rectangle_is_not_drawn() {
        let mut c = TestCanvas::new(8, 8);
        c.canvas()
            .draw_icon(&SQUARE, Rect::new(2, 2, 0, 6), FORE, BACK);
        c.canvas()
            .draw_icon(&SQUARE, Rect::new(2, 2, 6, 0), FORE, BACK);
        assert!(c.pixels().iter().all(|&p| p == 0), "something was drawn");
    }

    /// A shape with fewer than three points is skipped, not drawn wrong.
    #[test]
    fn a_degenerate_shape_is_skipped() {
        static LINE: Icon = Icon::new(&[
            Shape::fore(&[(0, 0), (100, 100)]),
            Shape::fore(&[(0, 40), (100, 40), (100, 60), (0, 60)]),
        ]);
        let mut c = TestCanvas::new(20, 20);
        c.canvas()
            .draw_icon(&LINE, Rect::new(0, 0, 20, 20), FORE, BACK);
        // The band still drew, so the skip did not abandon the rest.
        assert_eq!(c.at(10, 10), FORE.to_argb8888(), "the valid shape was lost");
    }
}
