//! Compositing 8-bit coverage masks.
//!
//! An anti-aliased glyph is a rectangle of coverage values, and drawing it is the
//! single most repeated operation a text-heavy panel performs. The M1 benches
//! measured the per-pixel path at **31 Mpx/s against 457 Mpx/s for the span
//! path** on a Pi 3 — fifteen times slower — and predicted that glyphs would be
//! where that gap gets paid. This is the code that stops it being paid.
//!
//! The trick is that a mask is mostly not partial. The interior of a glyph is
//! solid 255 and everything outside it is 0; only the rim is in between. So the
//! blitter walks each row in runs, sends solid runs through the span blend, skips
//! empty runs entirely, and pays the per-pixel cost only on the edge.

pub use denise::Mask;

use denise::Point;

use crate::blend::{Paint, blend_span};
use crate::canvas::Canvas;

impl Canvas<'_> {
    /// Composites `mask` in `color`, with its top-left corner at `at`.
    ///
    /// Coverage multiplies the paint's own alpha, so a half-transparent colour
    /// through a half-covered pixel lands at a quarter, which is what it should be.
    pub fn blit_mask(&mut self, at: Point, mask: &Mask<'_>, color: impl Into<Paint>) {
        let bounds = mask.bounds_at(at);
        let Some(visible) = self.visible(bounds) else {
            return;
        };
        let paint = color.into();
        if paint.alpha() == 0 {
            return;
        }
        let opaque = paint.alpha() == 255;

        for y in visible.y..visible.bottom() {
            let row = mask.row(y - at.y);
            let first = (visible.x - at.x) as usize;
            let last = (visible.right() - at.x) as usize;
            let mut x = first;
            while x < last {
                let value = row[x];
                if value == 0 {
                    x += 1;
                    continue;
                }
                // A run of identical coverage. Solid runs — the inside of the
                // glyph — go through the span blend; everything else is the rim,
                // which is narrow by construction.
                let mut end = x + 1;
                while end < last && row[end] == value {
                    end += 1;
                }
                let start_x = at.x + x as i32;
                let end_x = at.x + end as i32;
                if value == 255 && opaque {
                    if let Some(span) = self.row_span(y, start_x, end_x) {
                        blend_span(span, paint);
                    }
                } else {
                    let coverage = u32::from(value);
                    for px in start_x..end_x {
                        self.blend_at(px, y, paint, coverage);
                    }
                }
                x = end;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use denise::Rect;
    use crate::testing::TestCanvas;
    use denise::Color;

    /// A mask with a solid interior and a half-covered rim, like a real glyph.
    fn ring() -> ([u8; 36], i32, i32) {
        let mut data = [0u8; 36];
        for y in 0..6 {
            for x in 0..6 {
                let edge = x == 0 || y == 0 || x == 5 || y == 5;
                data[y * 6 + x] = if edge { 128 } else { 255 };
            }
        }
        (data, 6, 6)
    }

    #[test]
    fn geometry_is_validated() {
        let data = [0u8; 10];
        assert!(
            Mask::packed(&data, 4, 4).is_none(),
            "16 bytes needed, 10 given"
        );
        assert!(Mask::new(&data, 4, 2, 2).is_none(), "stride below width");
        assert!(Mask::packed(&data, 0, 4).is_none());
        assert!(Mask::packed(&data, 5, 2).is_some());
    }

    #[test]
    fn solid_coverage_paints_the_colour_exactly() {
        let data = [255u8; 16];
        let mask = Mask::packed(&data, 4, 4).expect("mask");
        let mut t = TestCanvas::new(8, 8);
        t.canvas().blit_mask(Point::new(2, 2), &mask, Color::WHITE);
        for y in 0..8usize {
            for x in 0..8usize {
                let inside = (2..6).contains(&x) && (2..6).contains(&y);
                let expected = if inside { 0xFFFF_FFFF } else { 0 };
                assert_eq!(t.pixels()[y * 8 + x], expected, "at {x},{y}");
            }
        }
    }

    #[test]
    fn partial_coverage_lands_between_the_endpoints() {
        let data = [128u8; 4];
        let mask = Mask::packed(&data, 2, 2).expect("mask");
        let mut t = TestCanvas::new(4, 4);
        t.canvas().blit_mask(Point::ZERO, &mask, Color::WHITE);
        let px = t.pixels()[0] & 0xFF;
        assert!(
            (120..=136).contains(&px),
            "half coverage over black should be about half white, got {px}"
        );
    }

    #[test]
    fn coverage_multiplies_the_paint_alpha() {
        let data = [128u8; 4];
        let mask = Mask::packed(&data, 2, 2).expect("mask");
        let mut t = TestCanvas::new(4, 4);
        t.canvas()
            .blit_mask(Point::ZERO, &mask, Color::rgba(255, 255, 255, 128));
        let px = t.pixels()[0] & 0xFF;
        assert!(
            (56..=72).contains(&px),
            "half alpha through half coverage should be about a quarter, got {px}"
        );
    }

    #[test]
    fn the_span_path_and_the_pixel_path_agree() {
        // The optimisation only holds if a solid run blitted as a span is
        // identical to the same run blitted pixel by pixel. Draw the same ring
        // twice, once opaque (span path) and once at alpha 254 (pixel path), and
        // require the interiors to differ by at most rounding.
        let (data, w, h) = ring();
        let mask = Mask::packed(&data, w, h).expect("mask");

        let mut span = TestCanvas::new(8, 8);
        span.canvas().blit_mask(Point::ZERO, &mask, Color::WHITE);
        let mut pixel = TestCanvas::new(8, 8);
        pixel
            .canvas()
            .blit_mask(Point::ZERO, &mask, Color::rgba(255, 255, 255, 254));

        for i in 0..64 {
            let a = span.pixels()[i] & 0xFF;
            let b = pixel.pixels()[i] & 0xFF;
            assert!(a.abs_diff(b) <= 2, "paths disagree at {i}: {a} vs {b}");
        }
    }

    #[test]
    fn zero_coverage_writes_nothing() {
        let data = [0u8; 16];
        let mask = Mask::packed(&data, 4, 4).expect("mask");
        let mut t = TestCanvas::new(8, 8);
        t.canvas().blit_mask(Point::ZERO, &mask, Color::WHITE);
        assert!(t.pixels().iter().all(|&p| p == 0));
    }

    #[test]
    fn a_transparent_paint_writes_nothing() {
        let data = [255u8; 16];
        let mask = Mask::packed(&data, 4, 4).expect("mask");
        let mut t = TestCanvas::new(8, 8);
        t.canvas()
            .blit_mask(Point::ZERO, &mask, Color::rgba(255, 0, 0, 0));
        assert!(t.pixels().iter().all(|&p| p == 0));
    }

    #[test]
    fn a_mask_is_clipped_like_everything_else() {
        let (data, w, h) = ring();
        let mask = Mask::packed(&data, w, h).expect("mask");
        let mut t = TestCanvas::new(16, 16);
        {
            let mut c = t.canvas();
            let mut clipped = c.with_clip(Rect::new(4, 4, 4, 4));
            // Straddles the clip on every side.
            clipped.blit_mask(Point::new(2, 2), &mask, Color::WHITE);
        }
        for y in 0..16usize {
            for x in 0..16usize {
                let inside = (4..8).contains(&x) && (4..8).contains(&y);
                if !inside {
                    assert_eq!(t.pixels()[y * 16 + x], 0, "drew past the clip at {x},{y}");
                }
            }
        }
        assert_ne!(t.pixels()[4 * 16 + 4], 0, "nothing was drawn at all");
    }

    #[test]
    fn a_mask_partly_off_the_top_left_draws_its_visible_part() {
        // Negative placement is what happens to a glyph with a left bearing at the
        // start of a clipped line, and indexing the mask by the clipped coordinate
        // rather than the placed one is the bug it produces.
        let data = [255u8; 16];
        let mask = Mask::packed(&data, 4, 4).expect("mask");
        let mut t = TestCanvas::new(8, 8);
        t.canvas()
            .blit_mask(Point::new(-2, -2), &mask, Color::WHITE);
        assert_eq!(t.pixels()[0], 0xFFFF_FFFF);
        assert_eq!(t.pixels()[8 + 1], 0xFFFF_FFFF);
        assert_eq!(t.pixels()[2 * 8 + 2], 0, "the mask is only 4 wide");
    }

    #[test]
    fn a_padded_stride_reads_the_right_rows() {
        // Atlas rows are slices of a wider buffer, so the stride is almost never
        // the glyph's width.
        let mut data = [7u8; 4 * 10];
        for y in 0..4 {
            for x in 0..3 {
                data[y * 10 + x] = 255;
            }
        }
        let mask = Mask::new(&data, 3, 4, 10).expect("mask");
        let mut t = TestCanvas::new(8, 8);
        t.canvas().blit_mask(Point::ZERO, &mask, Color::WHITE);
        for y in 0..4usize {
            for x in 0..8usize {
                let expected = if x < 3 { 0xFFFF_FFFF } else { 0 };
                assert_eq!(t.pixels()[y * 8 + x], expected, "at {x},{y}");
            }
        }
    }
}
