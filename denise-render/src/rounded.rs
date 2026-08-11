//! Anti-aliased rounded rectangles.
//!
//! The shape is evaluated analytically per scanline rather than by rendering a
//! path. For each row the corner arcs give an exact horizontal inset, and the only
//! approximation is that the arc is sampled at [`SUBSAMPLES`] sub-rows instead of
//! integrated exactly. That is enough at UI radii and costs a handful of integer
//! square roots per row — no path building, no allocation, no floating point.

use denise::Rect;

use crate::blend::{Paint, blend_span};
use crate::canvas::Canvas;

/// Sub-rows sampled per scanline. Four is the point where the near-horizontal top
/// of a corner stops looking stepped; eight is not visibly better.
const SUBSAMPLES: usize = 4;

/// Fractional bits in the fixed-point coordinates used here.
const FRAC_BITS: u32 = 8;
/// One whole pixel in fixed point.
const ONE: i32 = 1 << FRAC_BITS;
/// Vertical distance between sub-rows.
const SUB_STEP: i32 = ONE / SUBSAMPLES as i32;

/// Coordinates are clamped to this before entering fixed point, so a rectangle
/// placed absurdly far off-screen cannot overflow the shift. Any real surface is
/// orders of magnitude inside it.
const COORD_LIMIT: i32 = 1 << 22;

#[inline]
fn to_fx(v: i32) -> i32 {
    v.clamp(-COORD_LIMIT, COORD_LIMIT) << FRAC_BITS
}

#[inline]
fn floor_px(v: i32) -> i32 {
    v.div_euclid(ONE)
}

#[inline]
fn ceil_px(v: i32) -> i32 {
    (v + ONE - 1).div_euclid(ONE)
}

/// `sqrt(radius² - dy²)`, all in fixed point.
#[inline]
fn arc_half_width(radius: i32, dy: i32) -> i32 {
    let r2 = (radius as i64 * radius as i64) as u64;
    let d2 = (dy as i64 * dy as i64) as u64;
    r2.saturating_sub(d2).isqrt() as i32
}

/// Where a rounded rectangle starts and ends on one scanline, sampled at several
/// sub-rows and kept sub-pixel.
#[derive(Clone, Copy, Debug)]
struct Scan {
    left: [i32; SUBSAMPLES],
    right: [i32; SUBSAMPLES],
}

impl Scan {
    /// Computes the extent of `rect` with corner `radius` on row `y`.
    fn new(rect: Rect, radius: i32, y: i32) -> Self {
        let rad = to_fx(radius);
        let top = to_fx(rect.y);
        let bottom = to_fx(rect.bottom());
        let left_edge = to_fx(rect.x);
        let right_edge = to_fx(rect.right());

        let mut left = [0; SUBSAMPLES];
        let mut right = [0; SUBSAMPLES];

        for k in 0..SUBSAMPLES {
            let sy = to_fx(y) + k as i32 * SUB_STEP + SUB_STEP / 2;

            // Distance into whichever corner band this sub-row falls in. Outside
            // both bands the inset is zero and the row is a plain rectangle, which
            // is also what a zero radius gives everywhere.
            let from_top = sy - top;
            let from_bottom = bottom - sy;
            let dy = if from_top < rad {
                rad - from_top
            } else if from_bottom < rad {
                rad - from_bottom
            } else {
                0
            };

            let inset = if dy > 0 {
                rad - arc_half_width(rad, dy)
            } else {
                0
            };

            left[k] = left_edge + inset;
            right[k] = right_edge - inset;
        }

        Scan { left, right }
    }

    /// Coverage of pixel column `x`, `0..=255`.
    fn coverage(&self, x: i32) -> u32 {
        let px0 = to_fx(x);
        let px1 = px0 + ONE;
        let mut covered: i32 = 0;
        for k in 0..SUBSAMPLES {
            let l = self.left[k].max(px0);
            let r = self.right[k].min(px1);
            covered += (r - l).max(0);
        }
        // Rounded, not truncated. Truncation leaves a pixel that is 99.9% covered
        // reading as 254, which shows up as a hairline seam wherever a fill meets
        // the solid span next to it.
        let total = ONE as u32 * SUBSAMPLES as u32;
        ((covered as u32 * 255 + total / 2) / total).min(255)
    }

    #[inline]
    fn min_left(&self) -> i32 {
        *self.left.iter().min().expect("SUBSAMPLES > 0")
    }

    #[inline]
    fn max_left(&self) -> i32 {
        *self.left.iter().max().expect("SUBSAMPLES > 0")
    }

    #[inline]
    fn min_right(&self) -> i32 {
        *self.right.iter().min().expect("SUBSAMPLES > 0")
    }

    #[inline]
    fn max_right(&self) -> i32 {
        *self.right.iter().max().expect("SUBSAMPLES > 0")
    }
}

/// Coverage of a filled shape, or of the gap between two — a stroke.
struct Coverage<'a> {
    outer: &'a Scan,
    inner: Option<&'a Scan>,
}

impl Coverage<'_> {
    #[inline]
    fn at(&self, x: i32) -> u32 {
        let outer = self.outer.coverage(x);
        match self.inner {
            None => outer,
            Some(inner) => {
                let hole = inner.coverage(x);
                if hole == 0 {
                    outer
                } else {
                    // Correct rounding matters here: an off-by-one on a 1px stroke
                    // is a visible seam between the band and the solid interior.
                    (outer * (255 - hole) + 127) / 255
                }
            }
        }
    }
}

impl Canvas<'_> {
    /// Fills a rectangle with rounded corners, anti-aliased.
    ///
    /// `radius` is clamped to half the shorter side; a radius of zero is exactly
    /// [`Canvas::fill_rect`].
    pub fn fill_rounded_rect(&mut self, rect: Rect, radius: i32, color: impl Into<Paint>) {
        let paint = color.into();
        if paint.is_invisible() || rect.is_empty() {
            return;
        }
        let radius = radius.clamp(0, rect.width.min(rect.height) / 2);
        if radius == 0 {
            self.fill_rect(rect, paint);
            return;
        }
        let Some(vis) = self.visible(rect) else {
            return;
        };

        for y in vis.y..vis.bottom() {
            let outer = Scan::new(rect, radius, y);
            let cov = Coverage {
                outer: &outer,
                inner: None,
            };
            self.emit_run(
                y,
                floor_px(outer.min_left()),
                ceil_px(outer.max_right()),
                Some((ceil_px(outer.max_left()), floor_px(outer.min_right()))),
                &cov,
                paint,
            );
        }
    }

    /// Draws a rounded border of `thickness` pixels inside `rect`, anti-aliased.
    ///
    /// The inner radius follows the outer one so the band keeps a constant width
    /// around the corner.
    pub fn stroke_rounded_rect(
        &mut self,
        rect: Rect,
        radius: i32,
        thickness: i32,
        color: impl Into<Paint>,
    ) {
        let paint = color.into();
        let t = thickness.max(0);
        if t == 0 || rect.is_empty() || paint.is_invisible() {
            return;
        }
        let radius = radius.clamp(0, rect.width.min(rect.height) / 2);
        if t * 2 >= rect.width.min(rect.height) {
            self.fill_rounded_rect(rect, radius, paint);
            return;
        }

        let inner = Rect::new(
            rect.x + t,
            rect.y + t,
            rect.width - 2 * t,
            rect.height - 2 * t,
        );
        let inner_radius = (radius - t).max(0);

        let Some(vis) = self.visible(rect) else {
            return;
        };

        for y in vis.y..vis.bottom() {
            let outer = Scan::new(rect, radius, y);

            // Above and below the inner rectangle the whole row is stroke.
            if y < inner.y || y >= inner.bottom() {
                let cov = Coverage {
                    outer: &outer,
                    inner: None,
                };
                self.emit_run(
                    y,
                    floor_px(outer.min_left()),
                    ceil_px(outer.max_right()),
                    Some((ceil_px(outer.max_left()), floor_px(outer.min_right()))),
                    &cov,
                    paint,
                );
                continue;
            }

            let inner_scan = Scan::new(inner, inner_radius, y);
            let cov = Coverage {
                outer: &outer,
                inner: Some(&inner_scan),
            };

            // Two bands, skipping the interior entirely rather than blending it at
            // zero coverage. On a 1080p dialog that is the difference between
            // touching the border and touching the whole rectangle.
            self.emit_run(
                y,
                floor_px(outer.min_left()),
                ceil_px(inner_scan.max_left()),
                Some((ceil_px(outer.max_left()), floor_px(inner_scan.min_left()))),
                &cov,
                paint,
            );
            self.emit_run(
                y,
                floor_px(inner_scan.min_right()),
                ceil_px(outer.max_right()),
                Some((ceil_px(inner_scan.max_right()), floor_px(outer.min_right()))),
                &cov,
                paint,
            );
        }
    }

    /// Emits one horizontal run: anti-aliased fringe, solid span, anti-aliased
    /// fringe. `solid` is the half-open range that is known to be fully covered.
    fn emit_run(
        &mut self,
        y: i32,
        from: i32,
        to: i32,
        solid: Option<(i32, i32)>,
        cov: &Coverage<'_>,
        paint: Paint,
    ) {
        let clip = self.clip();
        let from = from.max(clip.x);
        let to = to.min(clip.right());
        if from >= to {
            return;
        }

        let (s0, s1) = match solid {
            Some((s0, s1)) if s0 < s1 => (s0.clamp(from, to), s1.clamp(from, to)),
            _ => (to, to),
        };

        for x in from..s0 {
            self.blend_at(x, y, paint, cov.at(x));
        }
        if s0 < s1
            && let Some(span) = self.row_span(y, s0, s1)
        {
            blend_span(span, paint);
        }
        for x in s1..to {
            self.blend_at(x, y, paint, cov.at(x));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestCanvas;
    use denise::Color;

    fn alpha_of(px: u32) -> u32 {
        // Opaque white on a black canvas, so any channel reads back as coverage.
        px & 0xFF
    }

    #[test]
    fn zero_radius_is_exactly_a_fill() {
        let mut rounded = TestCanvas::new(16, 16);
        rounded
            .canvas()
            .fill_rounded_rect(Rect::new(2, 2, 12, 12), 0, Color::WHITE);

        let mut square = TestCanvas::new(16, 16);
        square
            .canvas()
            .fill_rect(Rect::new(2, 2, 12, 12), Color::WHITE);

        assert_eq!(rounded.pixels(), square.pixels());
    }

    #[test]
    fn corners_are_cut_and_the_middle_is_not() {
        let mut t = TestCanvas::new(32, 32);
        t.canvas()
            .fill_rounded_rect(Rect::new(0, 0, 32, 32), 8, Color::WHITE);

        assert_eq!(alpha_of(t.at(0, 0)), 0, "corner must be empty");
        assert_eq!(alpha_of(t.at(16, 0)), 255, "top edge must be solid");
        assert_eq!(alpha_of(t.at(0, 16)), 255, "left edge must be solid");
        assert_eq!(alpha_of(t.at(16, 16)), 255, "centre must be solid");
        assert_eq!(alpha_of(t.at(31, 31)), 0, "corner must be empty");
    }

    #[test]
    fn corners_are_antialiased_not_stepped() {
        let radius = 10;
        let mut t = TestCanvas::new(32, 32);
        t.canvas()
            .fill_rounded_rect(Rect::new(0, 0, 32, 32), radius, Color::WHITE);

        // Not along the 45° diagonal: there the arc runs perpendicular to the walk
        // and genuinely does step from 0 to 255 in one pixel. The anti-aliasing
        // lives on the near-horizontal and near-vertical stretches of the arc, so
        // count partial pixels over the whole corner block instead.
        let partial = (0..radius)
            .flat_map(|y| (0..radius).map(move |x| (x, y)))
            .filter(|&(x, y)| (1..255).contains(&alpha_of(t.at(x, y))))
            .count();

        // An arc of radius 10 crosses roughly 2r pixel boundaries; well under half
        // that means the edge is being quantised, not anti-aliased.
        assert!(
            partial >= radius as usize,
            "only {partial} partially covered pixels in a radius-{radius} corner"
        );
    }

    #[test]
    fn coverage_reaches_both_extremes() {
        // Rounding must still let a fully covered pixel read as opaque and a fully
        // empty one as clear, rather than parking everything in between.
        let mut t = TestCanvas::new(32, 32);
        t.canvas()
            .fill_rounded_rect(Rect::new(0, 0, 32, 32), 10, Color::WHITE);
        assert_eq!(alpha_of(t.at(0, 0)), 0);
        assert_eq!(alpha_of(t.at(16, 16)), 255);
    }

    #[test]
    fn shape_is_symmetric() {
        let mut t = TestCanvas::new(32, 24);
        t.canvas()
            .fill_rounded_rect(Rect::new(0, 0, 32, 24), 7, Color::WHITE);

        for y in 0..24 {
            for x in 0..16 {
                assert_eq!(t.at(x, y), t.at(31 - x, y), "mirror at {x},{y}");
            }
        }
        for y in 0..12 {
            for x in 0..32 {
                assert_eq!(t.at(x, y), t.at(x, 23 - y), "flip at {x},{y}");
            }
        }
    }

    #[test]
    fn radius_is_clamped_to_half_the_shorter_side() {
        // A radius past the clamp is a stadium, not an error, and must stay inside.
        let mut t = TestCanvas::new(40, 20);
        t.canvas()
            .fill_rounded_rect(Rect::new(0, 0, 40, 20), 999, Color::WHITE);
        assert_eq!(alpha_of(t.at(20, 10)), 255);
        assert_eq!(alpha_of(t.at(0, 0)), 0);
        assert_eq!(alpha_of(t.at(20, 0)), 255);
    }

    #[test]
    fn fill_stays_inside_its_bounds() {
        let mut t = TestCanvas::new(32, 32);
        t.canvas()
            .fill_rounded_rect(Rect::new(8, 8, 16, 16), 4, Color::WHITE);
        for y in 0..32 {
            for x in 0..32 {
                let inside = (8..24).contains(&x) && (8..24).contains(&y);
                if !inside {
                    assert_eq!(t.at(x, y), 0, "spilled at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn clipping_a_rounded_fill_matches_the_unclipped_result() {
        let region = Rect::new(4, 4, 10, 10);

        let mut full = TestCanvas::new(32, 32);
        full.canvas()
            .fill_rounded_rect(Rect::new(2, 2, 24, 24), 6, Color::WHITE);

        let mut clipped = TestCanvas::new(32, 32);
        {
            let mut c = clipped.canvas();
            c.clip_to(region);
            c.fill_rounded_rect(Rect::new(2, 2, 24, 24), 6, Color::WHITE);
        }

        for y in 0..32 {
            for x in 0..32 {
                let expected = if region.contains(denise::Point::new(x, y)) {
                    full.at(x, y)
                } else {
                    0
                };
                assert_eq!(clipped.at(x, y), expected, "at {x},{y}");
            }
        }
    }

    #[test]
    fn stroke_leaves_the_interior_alone() {
        let mut t = TestCanvas::new(32, 32);
        t.canvas()
            .stroke_rounded_rect(Rect::new(2, 2, 28, 28), 8, 3, Color::WHITE);
        assert_eq!(alpha_of(t.at(16, 16)), 0, "interior must be untouched");
        assert_eq!(alpha_of(t.at(16, 2)), 255, "top band must be solid");
        assert_eq!(alpha_of(t.at(2, 16)), 255, "left band must be solid");
        assert_eq!(alpha_of(t.at(16, 6)), 0, "just inside the band");
    }

    #[test]
    fn stroke_covers_the_band_without_seams() {
        // Every pixel across the band, at the mid-height where the stroke is
        // vertical, must be fully covered. A gap here is the classic
        // outer-minus-inner rounding seam.
        let mut t = TestCanvas::new(40, 40);
        t.canvas()
            .stroke_rounded_rect(Rect::new(4, 4, 32, 32), 10, 4, Color::WHITE);
        for x in 4..8 {
            assert_eq!(alpha_of(t.at(x, 20)), 255, "seam at x={x}");
        }
    }

    #[test]
    fn stroke_thicker_than_the_shape_is_a_fill() {
        let mut t = TestCanvas::new(32, 32);
        t.canvas()
            .stroke_rounded_rect(Rect::new(4, 4, 16, 16), 4, 99, Color::WHITE);
        assert_eq!(alpha_of(t.at(12, 12)), 255);
        assert_eq!(alpha_of(t.at(0, 0)), 0);
    }

    #[test]
    fn stroke_alpha_does_not_double_up_anywhere() {
        // Two bands meeting must never composite the same pixel twice.
        let mut t = TestCanvas::new(40, 40);
        t.canvas().stroke_rounded_rect(
            Rect::new(4, 4, 32, 32),
            10,
            3,
            Color::rgba(255, 255, 255, 128),
        );
        let single = alpha_of(t.at(20, 4));
        for y in 0..40 {
            for x in 0..40 {
                assert!(
                    alpha_of(t.at(x, y)) <= single,
                    "double-composited at {x},{y}"
                );
            }
        }
    }

    #[test]
    fn degenerate_rects_do_not_panic() {
        let mut t = TestCanvas::new(16, 16);
        let mut c = t.canvas();
        c.fill_rounded_rect(Rect::new(0, 0, 1, 1), 4, Color::WHITE);
        c.fill_rounded_rect(Rect::new(0, 0, 0, 10), 4, Color::WHITE);
        c.fill_rounded_rect(Rect::new(-100, -100, 8, 8), 3, Color::WHITE);
        c.stroke_rounded_rect(Rect::new(0, 0, 2, 2), 1, 1, Color::WHITE);
        c.stroke_rounded_rect(Rect::new(1_000_000, 0, 8, 8), 3, 1, Color::WHITE);
    }
}
