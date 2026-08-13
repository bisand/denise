//! Stars, over an allocation-free scanline polygon filler.
//!
//! # Why there is a polygon filler behind one shape
//!
//! The crate documentation promises **no path builder**, and that promise is
//! kept: what is public here is [`Canvas::fill_star`], a shape in the same
//! sense [`Canvas::fill_circle`] is a shape. It computes its own vertices from
//! the same Q16 sine table the arcs use — no floating point, no `libm`.
//!
//! The general filler underneath it stays `pub(crate)`. A star is a ten-vertex
//! polygon and nothing else in the rasteriser draws one, so the machinery had
//! to exist; keeping it internal means a heart, an arrow or a hexagon can be
//! added the day one is genuinely wanted, without today committing to a public
//! path API that would then have to be supported forever.
//!
//! # No allocation
//!
//! This crate has neither `std` nor `alloc`, so a scanline's edge crossings
//! live in a fixed-size array on the stack — [`MAX_VERTICES`] of them, which is
//! an eight-pointed star and more than any UI shape needs. Coverage is
//! accumulated per pixel by summing sub-row overlaps rather than into a
//! buffer, exactly as the rounded rectangles do it.

use denise::{Point, Rect};

use crate::arc::{TURN, direction};
use crate::blend::{Paint, blend_span};
use crate::canvas::Canvas;
use crate::rounded::{COORD_LIMIT, ONE, SUB_STEP, SUBSAMPLES, ceil_px, floor_px, to_fx};

/// The most vertices a polygon may have, and so the most crossings one
/// scanline can produce. Sixteen points of a star is far past anything legible
/// at UI sizes.
pub(crate) const MAX_VERTICES: usize = 32;

/// Crossings of one sub-row, sorted ascending, in fixed point.
struct Crossings {
    xs: [i32; MAX_VERTICES],
    len: usize,
}

impl Crossings {
    /// Where the polygon's edges cross the horizontal line at `sy`.
    ///
    /// The rule is half-open in y — an edge counts when exactly one of its
    /// endpoints is at or above the line — which is what makes a vertex on the
    /// line count once rather than twice or not at all.
    fn at(points: &[(i32, i32)], sy: i32) -> Self {
        let mut xs = [0i32; MAX_VERTICES];
        let mut len = 0;
        for i in 0..points.len() {
            let (x0, y0) = points[i];
            let (x1, y1) = points[(i + 1) % points.len()];
            if (y0 <= sy) == (y1 <= sy) {
                continue;
            }
            // y1 != y0 here: the halves disagree, so the edge is not horizontal.
            let t = (sy - y0) as i64 * (x1 - x0) as i64 / (y1 - y0) as i64;
            let x = x0 as i64 + t;
            if len < MAX_VERTICES {
                xs[len] = x as i32;
                len += 1;
            }
        }
        // Insertion sort: at most MAX_VERTICES entries, and in practice two or
        // four. Nothing here is worth a better algorithm.
        for i in 1..len {
            let v = xs[i];
            let mut j = i;
            while j > 0 && xs[j - 1] > v {
                xs[j] = xs[j - 1];
                j -= 1;
            }
            xs[j] = v;
        }
        Self { xs, len }
    }

    /// How much of the pixel column `[px0, px0 + ONE)` this sub-row covers,
    /// in fixed-point units. Even-odd: the spans are consecutive pairs.
    fn overlap(&self, px0: i32) -> i32 {
        let px1 = px0 + ONE;
        let mut covered = 0;
        let mut k = 0;
        while k + 1 < self.len {
            let l = self.xs[k].max(px0);
            let r = self.xs[k + 1].min(px1);
            covered += (r - l).max(0);
            k += 2;
        }
        covered
    }
}

impl Canvas<'_> {
    /// Fills a polygon given in fixed point, by the even-odd rule.
    ///
    /// Vertices past [`MAX_VERTICES`] are ignored rather than drawn wrongly.
    pub(crate) fn fill_polygon_fx(&mut self, points: &[(i32, i32)], paint: Paint) {
        if points.len() < 3 || points.len() > MAX_VERTICES || paint.is_invisible() {
            return;
        }
        let (mut top, mut bottom) = (i32::MAX, i32::MIN);
        let (mut left, mut right) = (i32::MAX, i32::MIN);
        for &(x, y) in points {
            top = top.min(y);
            bottom = bottom.max(y);
            left = left.min(x);
            right = right.max(x);
        }
        let bbox = Rect::from_edges(
            floor_px(left),
            floor_px(top),
            ceil_px(right) + 1,
            ceil_px(bottom) + 1,
        );
        let Some(vis) = self.visible(bbox) else {
            return;
        };

        for y in vis.y..vis.bottom() {
            let mut rows = [const {
                Crossings {
                    xs: [0; MAX_VERTICES],
                    len: 0,
                }
            }; SUBSAMPLES];
            let mut simple = true;
            for (k, row) in rows.iter_mut().enumerate() {
                let sy = to_fx(y) + k as i32 * SUB_STEP + SUB_STEP / 2;
                *row = Crossings::at(points, sy);
                simple &= row.len == 2;
            }

            // When every sub-row crosses exactly twice — the body of the shape,
            // away from any notch — the fully covered run is between the
            // rightmost left edge and the leftmost right edge, and only the two
            // fringes have to be evaluated per pixel.
            let (solid0, solid1) = if simple {
                let l = rows.iter().map(|r| r.xs[0]).max().unwrap_or(0);
                let r = rows.iter().map(|r| r.xs[1]).min().unwrap_or(0);
                (ceil_px(l), floor_px(r))
            } else {
                (vis.right(), vis.right())
            };

            for x in vis.x..solid0.min(vis.right()) {
                self.blend_at(x, y, paint, coverage(&rows, x));
            }
            let (s0, s1) = (solid0.max(vis.x), solid1.min(vis.right()));
            if s0 < s1
                && let Some(span) = self.row_span(y, s0, s1)
            {
                blend_span(span, paint);
            }
            for x in s1.max(vis.x)..vis.right() {
                self.blend_at(x, y, paint, coverage(&rows, x));
            }
        }
    }

    /// Fills a star, anti-aliased.
    ///
    /// `points` is the number of spikes — five for the familiar one. Vertices
    /// alternate between `outer_radius` at the tips and `inner_radius` at the
    /// valleys, so the ratio between them is how pointed the star looks: about
    /// `0.38` of the outer radius is the classic pentagram, and an inner radius
    /// approaching the outer one is a polygon with `2 × points` sides.
    ///
    /// `rotation` is in the same binary turns as the arcs — see [`TURN`] — and
    /// zero puts a tip at twelve o'clock.
    ///
    /// Vertices are computed to sub-pixel precision even though the centre is
    /// whole pixels, which is what keeps a small star from looking chewed.
    ///
    /// # A five-pointed star is not exactly five-fold symmetric
    ///
    /// [`TURN`] is a power of two, so it divides exactly by two, four and eight
    /// and not by five. A five-pointed star's vertex angles are therefore each
    /// rounded to the nearest unit — at most half a unit in 65536, which is
    /// under a hundredth of a pixel at any radius a screen can show, and
    /// invisible. But it does mean a star rotated by `TURN / 5` is not the
    /// bit-identical picture, only the same one. Anything that needs exactness
    /// should compare against a tolerance rather than against pixels.
    pub fn fill_star(
        &mut self,
        centre: Point,
        outer_radius: i32,
        inner_radius: i32,
        points: u32,
        rotation: i32,
        color: impl Into<Paint>,
    ) {
        let paint = color.into();
        if points < 2 || outer_radius <= 0 || paint.is_invisible() {
            return;
        }
        let count = (points as usize) * 2;
        if count > MAX_VERTICES {
            return;
        }
        let outer = outer_radius.clamp(0, COORD_LIMIT) as i64;
        let inner = inner_radius.clamp(0, outer_radius) as i64;

        let mut vertices = [(0i32, 0i32); MAX_VERTICES];
        let (cx, cy) = (to_fx(centre.x), to_fx(centre.y));
        for (i, vertex) in vertices.iter_mut().enumerate().take(count) {
            // Rounded, not truncated: the spacing error is then at most half a
            // unit per vertex rather than a whole one.
            let step = (i as i64 * TURN as i64 + count as i64 / 2) / count as i64;
            let angle = rotation.wrapping_add(step as i32);
            let (dx, dy) = direction(angle);
            let r = if i % 2 == 0 { outer } else { inner };
            // The direction is Q16 and the target is 8.8, so a radius in whole
            // pixels lands sub-pixel after shifting off eight bits.
            *vertex = (
                cx + ((dx as i64 * r) >> 8) as i32,
                cy + ((dy as i64 * r) >> 8) as i32,
            );
        }
        self.fill_polygon_fx(&vertices[..count], paint);
    }
}

/// Coverage of pixel column `x` across every sub-row, `0..=255`.
fn coverage(rows: &[Crossings; SUBSAMPLES], x: i32) -> u32 {
    let px0 = to_fx(x);
    let mut covered: i32 = 0;
    for row in rows {
        covered += row.overlap(px0);
    }
    // Rounded rather than truncated, for the reason `Scan::coverage` gives: a
    // pixel that is 99.9% covered reading as 254 is a visible hairline seam.
    let total = ONE as u32 * SUBSAMPLES as u32;
    ((covered.max(0) as u32 * 255 + total / 2) / total).min(255)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestCanvas;
    use denise::Color;

    fn alpha_of(px: u32) -> u32 {
        px & 0xFF
    }

    /// The star's vertices in f64 pixel coordinates, computed independently of
    /// the fixed-point path — the oracle's own geometry.
    fn star_vertices(cx: f64, cy: f64, outer: f64, inner: f64, points: usize) -> Vec<(f64, f64)> {
        let count = points * 2;
        (0..count)
            .map(|i| {
                let a = i as f64 / count as f64 * core::f64::consts::TAU;
                let r = if i % 2 == 0 { outer } else { inner };
                (cx + a.sin() * r, cy - a.cos() * r)
            })
            .collect()
    }

    /// Even-odd point-in-polygon by ray casting, in floating point.
    fn inside(poly: &[(f64, f64)], x: f64, y: f64) -> bool {
        let mut hit = false;
        for i in 0..poly.len() {
            let (x0, y0) = poly[i];
            let (x1, y1) = poly[(i + 1) % poly.len()];
            if (y0 > y) != (y1 > y) && x < (x1 - x0) * (y - y0) / (y1 - y0) + x0 {
                hit = !hit;
            }
        }
        hit
    }

    /// The one test that would catch a wrong filler: coverage compared against
    /// 16x16 supersampling of an independently written point-in-polygon test.
    /// Hand-picked cases cannot see a systematic half-pixel shift; this can.
    #[test]
    fn star_coverage_matches_a_supersampled_oracle() {
        const N: i32 = 16;
        let (cx, cy, outer, inner) = (24, 24, 20, 8);
        let mut t = TestCanvas::new(48, 48);
        t.canvas()
            .fill_star(Point::new(cx, cy), outer, inner, 5, 0, Color::WHITE);

        let poly = star_vertices(cx as f64, cy as f64, outer as f64, inner as f64, 5);
        let mut worst = 0u32;
        for y in 0..48 {
            for x in 0..48 {
                let mut hits = 0;
                for sy in 0..N {
                    for sx in 0..N {
                        let px = x as f64 + (sx as f64 + 0.5) / N as f64;
                        let py = y as f64 + (sy as f64 + 0.5) / N as f64;
                        if inside(&poly, px, py) {
                            hits += 1;
                        }
                    }
                }
                let want = (hits * 255 / (N * N)) as u32;
                let got = alpha_of(t.at(x, y));
                worst = worst.max(got.abs_diff(want));
            }
        }
        // Four sub-rows against sixteen will differ on the fringes; a
        // systematic error would be far larger than this.
        assert!(worst <= 48, "worst pixel differs by {worst}");
    }

    /// Even-odd pairing walks crossings two at a time, so an odd count means a
    /// span that never closes — ink running to the edge of the clip. The rule
    /// that prevents it is the half-open comparison in `Crossings::at`, and the
    /// cases that test it are the ones nothing random will generate: a vertex
    /// exactly on a sub-row line, and a horizontal edge lying along one.
    ///
    /// Note that `[y0, y1)` and `(y0, y1]` are both correct here and a mutation
    /// swapping them is not detectable — either counts a vertex once. What must
    /// not happen is a rule that counts it twice or not at all.
    #[test]
    fn every_scanline_crosses_a_polygon_an_even_number_of_times() {
        let sub_row = |y: i32, k: i32| to_fx(y) + k * SUB_STEP + SUB_STEP / 2;

        // A triangle with its apex exactly on a sub-row, and a diamond with two
        // vertices there — then a rectangle whose top and bottom edges are
        // horizontal and lie exactly on sub-rows.
        let apex = sub_row(10, 0);
        let shapes: [&[(i32, i32)]; 3] = [
            &[
                (to_fx(10), apex),
                (to_fx(30), to_fx(30)),
                (to_fx(2), to_fx(28)),
            ],
            &[
                (to_fx(16), apex),
                (to_fx(28), sub_row(20, 2)),
                (to_fx(16), to_fx(34)),
                (to_fx(4), sub_row(20, 2)),
            ],
            &[
                (to_fx(4), apex),
                (to_fx(28), apex),
                (to_fx(28), sub_row(30, 1)),
                (to_fx(4), sub_row(30, 1)),
            ],
        ];

        for (n, shape) in shapes.iter().enumerate() {
            for y in 0..48 {
                for k in 0..SUBSAMPLES as i32 {
                    let c = Crossings::at(shape, sub_row(y, k));
                    assert!(
                        c.len.is_multiple_of(2),
                        "shape {n} at y={y} sub-row {k} crossed {} times",
                        c.len
                    );
                }
            }
        }
    }

    /// A horizontal edge has no crossing to compute, and computing one would
    /// divide by zero. The skip is load-bearing, not tidiness.
    #[test]
    fn a_horizontal_edge_never_divides_by_zero() {
        let mut t = TestCanvas::new(32, 32);
        let flat: &[(i32, i32)] = &[
            (to_fx(4), to_fx(8)),
            (to_fx(28), to_fx(8)),
            (to_fx(28), to_fx(20)),
            (to_fx(4), to_fx(20)),
        ];
        t.canvas().fill_polygon_fx(flat, Color::WHITE.into());
        assert_eq!(alpha_of(t.at(16, 14)), 255, "the interior must be filled");
        assert_eq!(alpha_of(t.at(16, 2)), 0, "and nothing above it");
    }

    #[test]
    fn a_star_has_its_tips_and_its_valleys() {
        let mut t = TestCanvas::new(64, 64);
        t.canvas()
            .fill_star(Point::new(32, 32), 28, 11, 5, 0, Color::WHITE);
        assert_eq!(alpha_of(t.at(32, 32)), 255, "the middle must be solid");
        // A tip at twelve o'clock, and empty just outside it.
        assert!(alpha_of(t.at(32, 8)) > 0, "no tip at twelve o'clock");
        assert_eq!(alpha_of(t.at(32, 2)), 0, "something past the tip");
        // The corners of the bounding box are outside every star.
        for (x, y) in [(4, 4), (59, 4), (4, 59), (59, 59)] {
            assert_eq!(alpha_of(t.at(x, y)), 0, "spilled at {x},{y}");
        }
    }

    #[test]
    fn a_star_stays_inside_its_radius_at_every_size() {
        for radius in [3, 8, 20, 60] {
            let mut t = TestCanvas::new(160, 160);
            t.canvas().fill_star(
                Point::new(80, 80),
                radius,
                radius * 2 / 5,
                5,
                0,
                Color::WHITE,
            );
            for y in 0..160i32 {
                for x in 0..160i32 {
                    if alpha_of(t.at(x, y)) == 0 {
                        continue;
                    }
                    let (dx, dy) = ((x - 80) as f64 + 0.5, (y - 80) as f64 + 0.5);
                    let d = (dx * dx + dy * dy).sqrt();
                    assert!(
                        d <= radius as f64 + 1.5,
                        "radius {radius}: ink at {x},{y} is {d} out"
                    );
                }
            }
        }
    }

    #[test]
    fn rotation_turns_the_star_and_a_full_turn_returns_it() {
        let draw = |rotation| {
            let mut t = TestCanvas::new(64, 64);
            t.canvas()
                .fill_star(Point::new(32, 32), 24, 10, 5, rotation, Color::WHITE);
            t
        };
        /// Pixels whose coverage differs by more than anti-aliasing noise.
        fn far_apart(a: &TestCanvas, b: &TestCanvas) -> usize {
            a.pixels()
                .iter()
                .zip(b.pixels())
                .filter(|&(&p, &q)| alpha_of(p).abs_diff(alpha_of(q)) > 24)
                .count()
        }

        let zero = draw(0);
        assert_eq!(
            zero.pixels(),
            draw(TURN).pixels(),
            "a full turn must be exactly identity"
        );

        // A fifth of a turn is the star's own symmetry — but TURN is a power of
        // two and does not divide by five, so this is the same star and not the
        // same pixels. See the note on `fill_star`.
        let fifth = far_apart(&zero, &draw(TURN / 5));
        assert!(fifth < 40, "five-fold symmetry is off by {fifth} pixels");

        // Half a step is a genuinely different orientation, and must look it —
        // otherwise the assertion above would be measuring nothing.
        let tenth = far_apart(&zero, &draw(TURN / 10));
        assert!(
            tenth > 10 * fifth.max(1),
            "half a step differs by only {tenth} against {fifth}"
        );
    }

    #[test]
    fn clipping_a_star_matches_the_unclipped_result() {
        let region = Rect::new(20, 20, 24, 24);
        let mut full = TestCanvas::new(64, 64);
        full.canvas()
            .fill_star(Point::new(32, 32), 26, 10, 5, 0, Color::WHITE);

        let mut clipped = TestCanvas::new(64, 64);
        {
            let mut c = clipped.canvas();
            c.clip_to(region);
            c.fill_star(Point::new(32, 32), 26, 10, 5, 0, Color::WHITE);
        }
        for y in 0..64 {
            for x in 0..64 {
                let expected = if region.contains(Point::new(x, y)) {
                    full.at(x, y)
                } else {
                    0
                };
                assert_eq!(clipped.at(x, y), expected, "at {x},{y}");
            }
        }
    }

    #[test]
    fn an_inner_radius_at_the_outer_one_is_a_convex_polygon() {
        // No spikes left: every vertex at the same radius is a 10-gon, which
        // must be solid all the way out rather than developing notches.
        let mut t = TestCanvas::new(64, 64);
        t.canvas()
            .fill_star(Point::new(32, 32), 20, 20, 5, 0, Color::WHITE);
        assert_eq!(alpha_of(t.at(32, 32)), 255);
        assert_eq!(alpha_of(t.at(32, 14)), 255, "a valley became a notch");
    }

    #[test]
    fn degenerate_stars_draw_nothing_and_nobody_panics() {
        let mut t = TestCanvas::new(32, 32);
        let mut c = t.canvas();
        c.fill_star(Point::new(16, 16), 0, 0, 5, 0, Color::WHITE);
        c.fill_star(Point::new(16, 16), -10, 4, 5, 0, Color::WHITE);
        c.fill_star(Point::new(16, 16), 10, 20, 5, 0, Color::WHITE);
        c.fill_star(Point::new(16, 16), 10, 4, 1, 0, Color::WHITE);
        c.fill_star(Point::new(16, 16), 10, 4, 99, 0, Color::WHITE);
        c.fill_star(Point::new(16, 16), 10, 4, 5, i32::MIN, Color::WHITE);
        c.fill_star(Point::new(1_000_000, 0), 10, 4, 5, 0, Color::WHITE);
        c.fill_star(Point::new(16, 16), i32::MAX, 4, 5, 0, Color::WHITE);
        c.fill_star(Point::new(16, 16), 10, 4, 5, 0, Color::rgba(255, 0, 0, 0));
    }

    #[test]
    fn an_inner_radius_larger_than_the_outer_is_clamped_not_inverted() {
        // Asking for a bigger valley than tip is a caller's arithmetic error;
        // clamping gives a polygon rather than a self-intersecting mess.
        let mut asked = TestCanvas::new(48, 48);
        asked
            .canvas()
            .fill_star(Point::new(24, 24), 16, 999, 5, 0, Color::WHITE);
        let mut clamped = TestCanvas::new(48, 48);
        clamped
            .canvas()
            .fill_star(Point::new(24, 24), 16, 16, 5, 0, Color::WHITE);
        assert_eq!(asked.pixels(), clamped.pixels());
    }

    #[test]
    fn alpha_never_doubles_up_anywhere() {
        // One pass over each pixel: a translucent star must nowhere composite
        // itself twice, which is what a fringe overlapping a solid run does.
        let mut t = TestCanvas::new(64, 64);
        t.canvas().fill_star(
            Point::new(32, 32),
            26,
            10,
            5,
            0,
            Color::rgba(255, 255, 255, 128),
        );
        for y in 0..64 {
            for x in 0..64 {
                assert!(alpha_of(t.at(x, y)) <= 128, "double-composited at {x},{y}");
            }
        }
    }
}
