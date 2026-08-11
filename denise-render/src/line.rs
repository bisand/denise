//! Anti-aliased lines.

use denise::{Point, Rect};

use crate::blend::Paint;
use crate::canvas::Canvas;

/// Fractional bits used when stepping along the major axis.
const FRAC_BITS: u32 = 8;
const ONE: i64 = 1 << FRAC_BITS;

/// Converts a fixed-point fraction (`0..ONE`) to coverage (`0..=255`).
#[inline]
fn coverage(frac: i64) -> u32 {
    (frac as u32 * 255) >> FRAC_BITS
}

impl Canvas<'_> {
    /// Draws a one-pixel line between two pixel centres, anti-aliased.
    ///
    /// Axis-aligned lines take an exact integer path and get no anti-aliasing,
    /// because a horizontal rule that has been softened to 97% grey looks like a
    /// rendering bug, not like quality. Everything else is a Wu-style two-pixel
    /// blend along the minor axis.
    ///
    /// Thickness is not a parameter. Borders come from [`Canvas::stroke_rect`] and
    /// [`Canvas::stroke_rounded_rect`]; thick arbitrary-angle lines are not
    /// something a UI toolkit needs before it can draw a chart, and that is not
    /// this milestone.
    pub fn draw_line(&mut self, a: Point, b: Point, color: impl Into<Paint>) {
        let paint = color.into();
        if paint.is_invisible() {
            return;
        }

        if a.y == b.y {
            let x0 = a.x.min(b.x);
            let x1 = a.x.max(b.x);
            self.fill_rect(Rect::from_edges(x0, a.y, x1 + 1, a.y + 1), paint);
            return;
        }
        if a.x == b.x {
            let y0 = a.y.min(b.y);
            let y1 = a.y.max(b.y);
            self.fill_rect(Rect::from_edges(a.x, y0, a.x + 1, y1 + 1), paint);
            return;
        }

        let dx = (b.x - a.x).abs();
        let dy = (b.y - a.y).abs();

        if dx >= dy {
            // Step x, blend the two pixels straddling the exact y.
            let (p, q) = if a.x <= b.x { (a, b) } else { (b, a) };
            let run = (q.x - p.x) as i64;
            let rise = (q.y - p.y) as i64;
            for x in p.x..=q.x {
                let t = (x - p.x) as i64;
                let y_fx = ((p.y as i64) << FRAC_BITS) + ((rise * t) << FRAC_BITS) / run;
                let y = y_fx.div_euclid(ONE) as i32;
                let frac = y_fx.rem_euclid(ONE);
                self.blend_at(x, y, paint, 255 - coverage(frac));
                self.blend_at(x, y + 1, paint, coverage(frac));
            }
        } else {
            let (p, q) = if a.y <= b.y { (a, b) } else { (b, a) };
            let run = (q.y - p.y) as i64;
            let rise = (q.x - p.x) as i64;
            for y in p.y..=q.y {
                let t = (y - p.y) as i64;
                let x_fx = ((p.x as i64) << FRAC_BITS) + ((rise * t) << FRAC_BITS) / run;
                let x = x_fx.div_euclid(ONE) as i32;
                let frac = x_fx.rem_euclid(ONE);
                self.blend_at(x, y, paint, 255 - coverage(frac));
                self.blend_at(x + 1, y, paint, coverage(frac));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestCanvas;
    use denise::Color;

    fn alpha_of(px: u32) -> u32 {
        px & 0xFF
    }

    #[test]
    fn horizontal_line_is_crisp() {
        let mut t = TestCanvas::new(16, 16);
        t.canvas()
            .draw_line(Point::new(2, 8), Point::new(12, 8), Color::WHITE);
        for x in 2..=12 {
            assert_eq!(alpha_of(t.at(x, 8)), 255, "at x={x}");
        }
        assert_eq!(t.at(1, 8), 0);
        assert_eq!(t.at(13, 8), 0);
        assert_eq!(t.at(8, 7), 0, "must not bleed to the row above");
        assert_eq!(t.at(8, 9), 0, "must not bleed to the row below");
    }

    #[test]
    fn vertical_line_is_crisp() {
        let mut t = TestCanvas::new(16, 16);
        t.canvas()
            .draw_line(Point::new(8, 2), Point::new(8, 12), Color::WHITE);
        for y in 2..=12 {
            assert_eq!(alpha_of(t.at(8, y)), 255, "at y={y}");
        }
        assert_eq!(t.at(7, 8), 0);
        assert_eq!(t.at(9, 8), 0);
    }

    #[test]
    fn exact_diagonal_is_crisp() {
        let mut t = TestCanvas::new(16, 16);
        t.canvas()
            .draw_line(Point::new(0, 0), Point::new(15, 15), Color::WHITE);
        for i in 0..16 {
            assert_eq!(alpha_of(t.at(i, i)), 255, "at {i},{i}");
        }
    }

    #[test]
    fn shallow_diagonal_is_antialiased() {
        let mut t = TestCanvas::new(32, 16);
        t.canvas()
            .draw_line(Point::new(0, 2), Point::new(31, 9), Color::WHITE);
        let partial = (0..32).any(|x| (0..16).any(|y| (1..255).contains(&alpha_of(t.at(x, y)))));
        assert!(partial, "a 7-in-31 slope must produce partial coverage");
    }

    #[test]
    fn every_column_of_a_shallow_line_is_covered() {
        // Coverage on the two straddling pixels must sum to a full pixel, so no
        // column of the line is ever thin or missing.
        let mut t = TestCanvas::new(32, 16);
        t.canvas()
            .draw_line(Point::new(0, 2), Point::new(31, 9), Color::WHITE);
        for x in 0..32 {
            let total: u32 = (0..16).map(|y| alpha_of(t.at(x, y))).sum();
            assert!((250..=255).contains(&total), "column {x} summed to {total}");
        }
    }

    #[test]
    fn direction_does_not_change_the_result() {
        let a = Point::new(3, 1);
        let b = Point::new(28, 13);

        let mut forward = TestCanvas::new(32, 16);
        forward.canvas().draw_line(a, b, Color::WHITE);

        let mut backward = TestCanvas::new(32, 16);
        backward.canvas().draw_line(b, a, Color::WHITE);

        assert_eq!(forward.pixels(), backward.pixels());
    }

    #[test]
    fn lines_are_clipped_not_wrapped() {
        let mut t = TestCanvas::with_stride(16, 16, 24);
        t.canvas()
            .draw_line(Point::new(-40, -10), Point::new(60, 30), Color::WHITE);
        // Nothing may land in the stride padding, which is where an unclipped
        // negative x would wrap to.
        for row in t.pixels().chunks(24) {
            assert!(row[16..].iter().all(|&p| p == 0));
        }
    }

    #[test]
    fn single_point_line_does_not_panic() {
        let mut t = TestCanvas::new(8, 8);
        t.canvas()
            .draw_line(Point::new(4, 4), Point::new(4, 4), Color::WHITE);
        assert_eq!(alpha_of(t.at(4, 4)), 255);
    }
}
