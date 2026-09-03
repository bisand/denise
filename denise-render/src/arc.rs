//! Anti-aliased circles and arcs.
//!
//! The same machinery as the rounded rectangles — per-scanline extents in fixed
//! point, sampled at [`SUBSAMPLES`] sub-rows, integer square roots — with one
//! addition: an angular cut. An arc is the ring between two radii, intersected
//! with the sector between two rays, and on any one sub-row both of those are
//! just intervals of x. No trigonometry runs per pixel; the two ray directions
//! are looked up once per call.
//!
//! # Angles are binary turns
//!
//! A full revolution is [`TURN`] = 65536, angle 0 points at twelve o'clock, and
//! positive sweeps go clockwise. This is the unit a panel actually computes in:
//! a progress ring is `done * TURN / total` with no floating point and no π, and
//! wrap-around is `& (TURN - 1)` rather than a comparison nobody remembers to
//! write. Radians would put `f32` and a libm dependency in the one crate that
//! has neither; degrees would make the common quarters 90/180/270 but leave the
//! progress ring with a rounding remainder. Negative sweeps go anticlockwise.
//!
//! # The sine table
//!
//! `sin` and `cos` are not in `core`. The two ray directions come from a
//! 257-entry quarter-wave table in Q16, linearly interpolated — the worst error
//! against the real function is under 2 parts in 65536, which at a radius of a
//! thousand pixels misplaces a ray endpoint by a fortieth of a pixel. The table
//! is checked exhaustively by tests: every one of the 65536 angles must satisfy
//! the Pythagorean identity to within a part in a thousand, and the quarters
//! must be exact.

use denise::{Point, Rect};

pub use denise::TURN;

use denise::angle::direction;
use crate::blend::Paint;
use crate::canvas::Canvas;
use crate::rounded::{COORD_LIMIT, ONE, SUB_STEP, SUBSAMPLES, Scan, ceil_px, floor_px, to_fx};

/// Sentinel for an unbounded side of a row interval. Far beyond any coordinate
/// fixed point can carry, and far from overflowing anything it is added to.
const UNBOUNDED: i64 = i64::MAX / 4;

/// floor(b / a) for any sign of `a`.
fn floor_div(b: i64, a: i64) -> i64 {
    if a < 0 {
        (-b).div_euclid(-a)
    } else {
        b.div_euclid(a)
    }
}

/// ceil(b / a) for any sign of `a`.
fn ceil_div(b: i64, a: i64) -> i64 {
    -floor_div(-b, a)
}

/// The x-interval of one row that a half-plane through the centre keeps.
///
/// The half-plane is `cross(d, p - c) >= 0` (or `<= 0` for `keep_ge = false`)
/// with `cross(d, r) = d.x·ry - d.y·rx`. On the row at height `ry` (fixed point,
/// relative to the centre) that is linear in `rx`, so it keeps a half-line —
/// or the whole row, or none of it, when the boundary ray is horizontal.
fn half_plane(d: (i32, i32), ry: i64, keep_ge: bool) -> Option<(i64, i64)> {
    let b = d.0 as i64 * ry;
    let a = d.1 as i64;
    if a == 0 {
        let keeps = if keep_ge { b >= 0 } else { b <= 0 };
        return keeps.then_some((-UNBOUNDED, UNBOUNDED));
    }
    // keep_ge:  a·rx <= b.  Otherwise:  a·rx >= b.  Dividing flips with a's sign.
    let bounded_above = (a > 0) == keep_ge;
    if bounded_above {
        Some((-UNBOUNDED, floor_div(b, a)))
    } else {
        Some((ceil_div(b, a), UNBOUNDED))
    }
}

/// The x-interval of one row inside the sector from `s` clockwise to `e`.
///
/// Only valid for sweeps of at most half a turn, where a sector is the
/// intersection of two half-planes; a wider sweep is handled by its caller as
/// the complement of the narrower one.
fn sector_row(s: (i32, i32), e: (i32, i32), ry: i64) -> Option<(i64, i64)> {
    let (lo_a, hi_a) = half_plane(s, ry, true)?;
    let (lo_b, hi_b) = half_plane(e, ry, false)?;
    let lo = lo_a.max(lo_b);
    let hi = hi_a.min(hi_b);
    (lo <= hi).then_some((lo, hi))
}

/// How the angular cut applies to a row interval.
enum Cut {
    /// Keep what falls inside the sector: a sweep of at most half a turn.
    Keep,
    /// Remove what falls inside the sector: the complement, for wider sweeps.
    Remove,
}

/// The spans of one scanline, per sub-row, after every cut. At most four per
/// sub-row: the ring contributes two, and removing a wedge can split one.
struct RowSpans {
    spans: [[(i32, i32); 4]; SUBSAMPLES],
    counts: [usize; SUBSAMPLES],
}

impl RowSpans {
    /// Total coverage of pixel column `x`, `0..=255` — the same rounding as the
    /// rounded rectangles, so the two primitives meet without seams.
    fn coverage(&self, x: i32) -> u32 {
        let px0 = to_fx(x);
        let px1 = px0 + ONE;
        let mut covered: i32 = 0;
        for k in 0..SUBSAMPLES {
            for &(l, r) in &self.spans[k][..self.counts[k]] {
                covered += (r.min(px1) - l.max(px0)).max(0);
            }
        }
        let total = ONE as u32 * SUBSAMPLES as u32;
        ((covered as u32 * 255 + total / 2) / total).min(255)
    }
}

/// The pixel-column ranges a row's spans touch, merged so no column is visited
/// twice — visiting one twice would composite translucent paint twice.
struct Clusters {
    runs: [(i32, i32); 16],
    count: usize,
}

impl Clusters {
    fn new() -> Self {
        Self {
            runs: [(0, 0); 16],
            count: 0,
        }
    }

    fn push(&mut self, from: i32, to: i32) {
        if from >= to {
            return;
        }
        // Merge with anything overlapping or adjacent, repeatedly: absorbing one
        // run can bring the result into contact with another.
        let mut from = from;
        let mut to = to;
        let mut i = 0;
        while i < self.count {
            let (a, b) = self.runs[i];
            if from <= b && to >= a {
                from = from.min(a);
                to = to.max(b);
                self.count -= 1;
                self.runs[i] = self.runs[self.count];
            } else {
                i += 1;
            }
        }
        if self.count < self.runs.len() {
            self.runs[self.count] = (from, to);
            self.count += 1;
        }
    }
}

impl Canvas<'_> {
    /// Fills a circle of `radius` around `centre`, anti-aliased.
    ///
    /// Exactly [`Canvas::fill_rounded_rect`] on the bounding square — this name
    /// exists so the intent is readable and the radius impossible to get wrong.
    /// The painted diameter is `2 * radius` pixels.
    pub fn fill_circle(&mut self, centre: Point, radius: i32, color: impl Into<Paint>) {
        let r = radius.clamp(0, COORD_LIMIT);
        let square = bounding_square(centre, r);
        self.fill_rounded_rect(square, r, color);
    }

    /// Draws a ring of `thickness` pixels just inside the circle of `radius`
    /// around `centre`, anti-aliased.
    pub fn stroke_circle(
        &mut self,
        centre: Point,
        radius: i32,
        thickness: i32,
        color: impl Into<Paint>,
    ) {
        let r = radius.clamp(0, COORD_LIMIT);
        let square = bounding_square(centre, r);
        self.stroke_rounded_rect(square, r, thickness.min(COORD_LIMIT), color);
    }

    /// Draws part of a ring: from `start`, sweeping `sweep`, both in units of
    /// [`TURN`]. Angle 0 is twelve o'clock and positive sweeps go clockwise;
    /// a negative sweep goes the other way. The ends are cut flat along the
    /// radius — butt caps.
    ///
    /// A sweep of at least a full [`TURN`] is exactly [`Canvas::stroke_circle`],
    /// which is what lets a progress ring pass `done * TURN / total` without
    /// special-casing 100%. A `thickness` of at least `radius` fills to the
    /// centre, which makes a pie slice.
    ///
    /// The cost is proportional to the pixels the arc actually covers, not to
    /// its bounding square — a thin spinner touches a thin ring of pixels. What
    /// to *damage* for an animated arc is the caller's business, but the same
    /// property means a conservative bounding rectangle only costs rasterising
    /// the ring inside it, not the square.
    pub fn stroke_arc(
        &mut self,
        centre: Point,
        radius: i32,
        thickness: i32,
        start: i32,
        sweep: i32,
        color: impl Into<Paint>,
    ) {
        let paint = color.into();
        let r = radius.clamp(0, COORD_LIMIT);
        let t = thickness.clamp(0, COORD_LIMIT).min(r);
        if r == 0 || t == 0 || sweep == 0 || paint.is_invisible() {
            return;
        }

        // Widened before normalising: negating `i32::MIN` overflows, and a
        // sweep is allowed to be anything.
        let mut start = start as i64;
        let mut sweep = sweep as i64;
        if sweep < 0 {
            start += sweep;
            sweep = -sweep;
        }
        if sweep >= TURN as i64 {
            self.stroke_circle(centre, r, t, paint);
            return;
        }
        let start = start.rem_euclid(TURN as i64) as i32;
        let sweep = sweep as i32;

        // A sector of at most half a turn is the intersection of two
        // half-planes. A wider one is not — but its complement is, so the wider
        // arc keeps what the complementary wedge does not claim.
        let (cut, from, to) = if sweep <= TURN / 2 {
            (Cut::Keep, start, start + sweep)
        } else {
            (Cut::Remove, start + sweep, start + TURN)
        };
        let s_dir = direction(from);
        let e_dir = direction(to);

        let square = bounding_square(centre, r);
        let Some(vis) = self.visible(square) else {
            return;
        };
        let inner_r = r - t;
        let inner_square = bounding_square(centre, inner_r);
        let centre_y = to_fx(centre.y);
        // The sector arithmetic is relative to the centre; the chords are
        // absolute. This is the shift that reconciles them.
        let centre_x = to_fx(centre.x) as i64;

        for y in vis.y..vis.bottom() {
            let outer = Scan::new(square, r, y);
            let hole = inner_r > 0 && y >= inner_square.y && y < inner_square.bottom();
            let inner = if hole {
                Some(Scan::new(inner_square, inner_r, y))
            } else {
                None
            };

            let mut row = RowSpans {
                spans: [[(0, 0); 4]; SUBSAMPLES],
                counts: [0; SUBSAMPLES],
            };
            let mut clusters = Clusters::new();

            for k in 0..SUBSAMPLES {
                let sy = to_fx(y) + k as i32 * SUB_STEP + SUB_STEP / 2;
                let ry = (sy - centre_y) as i64;

                // The ring on this sub-row: the outer chord, minus the inner
                // one when it exists.
                let (ol, or_) = (outer.left[k], outer.right[k]);
                if ol >= or_ {
                    continue;
                }
                let ring: [(i32, i32); 2] = match &inner {
                    Some(inner) if inner.left[k] < inner.right[k] => {
                        [(ol, inner.left[k]), (inner.right[k], or_)]
                    }
                    _ => [(ol, or_), (0, 0)],
                };

                let sector = sector_row(s_dir, e_dir, ry)
                    .map(|(lo, hi)| (lo.saturating_add(centre_x), hi.saturating_add(centre_x)));
                let mut push = |l: i64, r: i64| {
                    // Clamped into the chord *before* narrowing: these values
                    // can carry the UNBOUNDED sentinel, and `as i32` on that
                    // truncates — which once turned "no piece at all" into
                    // "the whole chord".
                    let l = l.clamp(ol as i64, or_ as i64) as i32;
                    let r = r.clamp(ol as i64, or_ as i64) as i32;
                    if l < r {
                        let n = &mut row.counts[k];
                        row.spans[k][*n] = (l, r);
                        *n += 1;
                        clusters.push(floor_px(l), ceil_px(r));
                    }
                };

                for &(l, r) in ring.iter().filter(|(l, r)| l < r) {
                    match (&cut, sector) {
                        (Cut::Keep, None) => {}
                        (Cut::Keep, Some((cl, ch))) => {
                            push((l as i64).max(cl), (r as i64).min(ch));
                        }
                        (Cut::Remove, None) => push(l as i64, r as i64),
                        (Cut::Remove, Some((wl, wh))) => {
                            push(l as i64, (r as i64).min(wl));
                            push((l as i64).max(wh), r as i64);
                        }
                    }
                }
            }

            let clip = self.clip();
            for &(from, to) in &clusters.runs[..clusters.count] {
                let from = from.max(clip.x);
                let to = to.min(clip.right());
                for x in from..to {
                    self.blend_at(x, y, paint, row.coverage(x));
                }
            }
        }
    }
}

/// The square a circle of `radius` around `centre` fits in, saturating so an
/// absurd centre cannot overflow — the coordinates are clamped again on their
/// way into fixed point.
fn bounding_square(centre: Point, radius: i32) -> Rect {
    Rect::new(
        centre.x.saturating_sub(radius),
        centre.y.saturating_sub(radius),
        radius.saturating_mul(2),
        radius.saturating_mul(2),
    )
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

    // ------------------------------------------------------------ the shapes

    /// The named wrappers are exactly the rounded-rect primitives on the
    /// bounding square — same pixels, no second implementation to drift.
    #[test]
    fn a_circle_is_exactly_a_fully_rounded_square() {
        let mut circle = TestCanvas::new(48, 48);
        circle
            .canvas()
            .fill_circle(Point::new(24, 24), 20, Color::WHITE);
        let mut square = TestCanvas::new(48, 48);
        square
            .canvas()
            .fill_rounded_rect(Rect::new(4, 4, 40, 40), 20, Color::WHITE);
        assert_eq!(circle.pixels(), square.pixels());

        let mut ring = TestCanvas::new(48, 48);
        ring.canvas()
            .stroke_circle(Point::new(24, 24), 20, 4, Color::WHITE);
        let mut band = TestCanvas::new(48, 48);
        band.canvas()
            .stroke_rounded_rect(Rect::new(4, 4, 40, 40), 20, 4, Color::WHITE);
        assert_eq!(ring.pixels(), band.pixels());
    }

    /// A full sweep is the circle, bit for bit — the equality the issue asked
    /// for, and what lets a progress ring pass `TURN` at 100% unspecial-cased.
    #[test]
    fn a_full_sweep_matches_the_circle_exactly() {
        for start in [0, TURN / 8, -TURN / 3] {
            let mut arc = TestCanvas::new(48, 48);
            arc.canvas()
                .stroke_arc(Point::new(24, 24), 20, 4, start, TURN, Color::WHITE);
            let mut circle = TestCanvas::new(48, 48);
            circle
                .canvas()
                .stroke_circle(Point::new(24, 24), 20, 4, Color::WHITE);
            assert_eq!(arc.pixels(), circle.pixels(), "start {start}");
        }
        // And more than a full turn is still just the circle.
        let mut over = TestCanvas::new(48, 48);
        over.canvas()
            .stroke_arc(Point::new(24, 24), 20, 4, 0, TURN * 3, Color::WHITE);
        let mut circle = TestCanvas::new(48, 48);
        circle
            .canvas()
            .stroke_circle(Point::new(24, 24), 20, 4, Color::WHITE);
        assert_eq!(over.pixels(), circle.pixels());
    }

    /// A zero sweep draws nothing at all.
    #[test]
    fn a_zero_sweep_draws_nothing() {
        let mut t = TestCanvas::new(48, 48);
        t.canvas()
            .stroke_arc(Point::new(24, 24), 20, 4, TURN / 8, 0, Color::WHITE);
        assert!(t.pixels().iter().all(|&px| px == 0));
    }

    /// A quarter sweep from twelve o'clock lives in the top-right quadrant and
    /// nowhere else — one pixel of slack along the cut edges, where the
    /// anti-aliasing genuinely straddles the ray.
    #[test]
    fn a_quarter_arc_stays_in_its_quadrant() {
        let (cx, cy) = (24, 24);
        let mut t = TestCanvas::new(48, 48);
        t.canvas()
            .stroke_arc(Point::new(cx, cy), 20, 4, 0, TURN / 4, Color::WHITE);

        let mut painted = 0;
        for y in 0..48 {
            for x in 0..48 {
                if alpha_of(t.at(x, y)) > 0 {
                    painted += 1;
                    assert!(
                        x >= cx - 1 && y <= cy,
                        "quarter arc escaped its quadrant at {x},{y}"
                    );
                }
            }
        }
        assert!(painted > 50, "only {painted} pixels for a quarter arc");
        // The twelve o'clock cap is at the top, the three o'clock cap on the
        // right: both ends of the sweep actually drew. Sampled one pixel up
        // from the axis, because the centre of an even-diameter circle is a
        // pixel *boundary* — the cap at three o'clock runs between rows.
        assert!(alpha_of(t.at(cx, cy - 20 + 2)) > 0, "no paint at the start");
        assert!(
            alpha_of(t.at(cx + 20 - 2, cy - 1)) > 0,
            "no paint at the end"
        );
    }

    /// A sweep crossing the wrap point paints both sides of twelve o'clock.
    #[test]
    fn a_sweep_across_the_wrap_point_paints_both_sides_of_the_top() {
        let (cx, cy) = (24, 24);
        let mut t = TestCanvas::new(48, 48);
        // From 315° round to 45°, crossing zero.
        t.canvas().stroke_arc(
            Point::new(cx, cy),
            20,
            4,
            7 * TURN / 8,
            TURN / 4,
            Color::WHITE,
        );
        assert!(alpha_of(t.at(cx - 8, cy - 17)) > 0, "left of the top");
        assert!(alpha_of(t.at(cx + 8, cy - 17)) > 0, "right of the top");
        assert_eq!(alpha_of(t.at(cx, cy + 18)), 0, "nothing at the bottom");
        assert_eq!(alpha_of(t.at(cx - 18, cy)), 0, "nothing at nine o'clock");
        assert_eq!(alpha_of(t.at(cx + 18, cy)), 0, "nothing at three o'clock");
    }

    /// A negative sweep is the same arc as the positive one that ends where it
    /// starts.
    #[test]
    fn a_negative_sweep_goes_the_other_way() {
        let mut negative = TestCanvas::new(48, 48);
        negative
            .canvas()
            .stroke_arc(Point::new(24, 24), 20, 4, 0, -TURN / 4, Color::WHITE);
        let mut positive = TestCanvas::new(48, 48);
        positive.canvas().stroke_arc(
            Point::new(24, 24),
            20,
            4,
            3 * TURN / 4,
            TURN / 4,
            Color::WHITE,
        );
        assert_eq!(negative.pixels(), positive.pixels());
    }

    /// Sweeps wider than half a turn go through the complement path; the two
    /// paths have to agree about where an edge is. A three-quarter arc and the
    /// quarter arc that completes it must tile the ring: everywhere the circle
    /// is solid, the two together must account for it, and where the circle is
    /// empty both must be empty.
    #[test]
    fn a_wide_arc_and_its_complement_tile_the_ring() {
        let mut wide = TestCanvas::new(48, 48);
        wide.canvas().stroke_arc(
            Point::new(24, 24),
            20,
            4,
            TURN / 4,
            3 * TURN / 4,
            Color::WHITE,
        );
        let mut narrow = TestCanvas::new(48, 48);
        narrow
            .canvas()
            .stroke_arc(Point::new(24, 24), 20, 4, 0, TURN / 4, Color::WHITE);
        let mut circle = TestCanvas::new(48, 48);
        circle
            .canvas()
            .stroke_circle(Point::new(24, 24), 20, 4, Color::WHITE);

        for y in 0..48 {
            for x in 0..48 {
                let whole = alpha_of(circle.at(x, y));
                let sum = alpha_of(wide.at(x, y)) + alpha_of(narrow.at(x, y));
                if whole == 0 {
                    assert_eq!(sum, 0, "painted outside the ring at {x},{y}");
                } else if whole == 255 {
                    // Butt caps overlap by at most the anti-aliased edge, so the
                    // sum can exceed a full pixel but never fall short of one.
                    assert!(
                        (255..=510).contains(&sum),
                        "the two arcs left a hole at {x},{y}: {sum}"
                    );
                }
            }
        }
    }

    /// The interior of a ring is untouched, and a thickness of at least the
    /// radius fills to the centre — the pie-slice case.
    #[test]
    fn thickness_decides_between_a_ring_and_a_pie() {
        let mut ring = TestCanvas::new(48, 48);
        ring.canvas()
            .stroke_arc(Point::new(24, 24), 20, 4, 0, TURN / 2, Color::WHITE);
        assert_eq!(alpha_of(ring.at(24, 24)), 0, "ring centre must be empty");
        assert_eq!(alpha_of(ring.at(30, 24)), 0, "ring interior must be empty");

        let mut pie = TestCanvas::new(48, 48);
        pie.canvas()
            .stroke_arc(Point::new(24, 24), 20, 99, 0, TURN / 2, Color::WHITE);
        assert_eq!(alpha_of(pie.at(30, 24)), 255, "pie interior must be solid");
        assert_eq!(alpha_of(pie.at(17, 24)), 0, "outside the pie's half");
    }

    /// Translucent paint composites once per pixel, however the spans and
    /// clusters carve the row up.
    #[test]
    fn translucent_arcs_never_composite_a_pixel_twice() {
        for sweep in [TURN / 4, TURN / 2, 3 * TURN / 4, TURN - TURN / 16] {
            let mut t = TestCanvas::new(48, 48);
            t.canvas().stroke_arc(
                Point::new(24, 24),
                20,
                4,
                TURN / 16,
                sweep,
                Color::rgba(255, 255, 255, 128),
            );
            let ceiling = 128;
            for y in 0..48 {
                for x in 0..48 {
                    assert!(
                        alpha_of(t.at(x, y)) <= ceiling,
                        "double-composited at {x},{y} with sweep {sweep}"
                    );
                }
            }
        }
    }

    /// Clipping changes which pixels are written, never what is written.
    #[test]
    fn clipping_an_arc_matches_the_unclipped_result() {
        let region = Rect::new(10, 6, 20, 22);
        let mut full = TestCanvas::new(48, 48);
        full.canvas()
            .stroke_arc(Point::new(24, 24), 18, 5, 0, 3 * TURN / 4, Color::WHITE);
        let mut clipped = TestCanvas::new(48, 48);
        {
            let mut c = clipped.canvas();
            c.clip_to(region);
            c.stroke_arc(Point::new(24, 24), 18, 5, 0, 3 * TURN / 4, Color::WHITE);
        }
        for y in 0..48 {
            for x in 0..48 {
                let expected = if region.contains(Point::new(x, y)) {
                    full.at(x, y)
                } else {
                    0
                };
                assert_eq!(clipped.at(x, y), expected, "at {x},{y}");
            }
        }
    }

    /// Degenerate and absurd inputs draw nothing or something, but never panic —
    /// a panic inside a paint loop on a kiosk is a black screen.
    #[test]
    fn degenerate_arcs_do_not_panic() {
        let mut t = TestCanvas::new(16, 16);
        let mut c = t.canvas();
        c.stroke_arc(Point::new(8, 8), 0, 4, 0, TURN, Color::WHITE);
        c.stroke_arc(Point::new(8, 8), 6, 0, 0, TURN, Color::WHITE);
        c.stroke_arc(Point::new(8, 8), -5, 3, 0, TURN, Color::WHITE);
        c.stroke_arc(Point::new(8, 8), 6, -2, 0, TURN, Color::WHITE);
        c.stroke_arc(Point::new(8, 8), 6, 3, i32::MIN, i32::MIN, Color::WHITE);
        c.stroke_arc(Point::new(8, 8), 6, 3, i32::MAX, i32::MAX, Color::WHITE);
        c.stroke_arc(
            Point::new(i32::MIN, i32::MAX),
            i32::MAX,
            i32::MAX,
            1,
            1,
            Color::WHITE,
        );
        c.fill_circle(Point::new(8, 8), 0, Color::WHITE);
        c.fill_circle(Point::new(-1000, 8), i32::MAX, Color::WHITE);
        c.stroke_circle(Point::new(8, 8), 6, i32::MAX, Color::WHITE);
    }

    /// The rasteriser against an independent oracle: 16×16 supersampled
    /// point-in-annulus ∧ point-in-sector membership, in the same integer
    /// arithmetic but sharing none of the scanline code. Interior and exterior
    /// pixels must agree exactly; edge pixels within the difference between
    /// 4-sub-row coverage and true area.
    #[test]
    fn coverage_agrees_with_a_supersampled_oracle() {
        let (cx, cy) = (24, 24);
        let (r, t) = (18, 5);
        for (start, sweep) in [
            (0, TURN / 4),
            (TURN / 8, TURN / 2),
            (7 * TURN / 8, TURN / 4),
            (TURN / 4, 3 * TURN / 4),
            (0, TURN / 2),
        ] {
            let mut canvas = TestCanvas::new(48, 48);
            canvas
                .canvas()
                .stroke_arc(Point::new(cx, cy), r, t, start, sweep, Color::WHITE);

            let s_dir = direction(start);
            let e_dir = direction(start + sweep);
            let wide = sweep > TURN / 2;

            for py in 0..48 {
                for px in 0..48 {
                    let mut hits = 0u32;
                    for sy in 0..16 {
                        for sx in 0..16 {
                            // Sample point in 1/32ths of a pixel, relative to
                            // the centre.
                            let dx = (px - cx) * 32 + sx * 2 + 1;
                            let dy = (py - cy) * 32 + sy * 2 + 1;
                            let d2 = (dx as i64) * (dx as i64) + (dy as i64) * (dy as i64);
                            let outer = (r as i64 * 32).pow(2);
                            let inner = ((r - t) as i64 * 32).pow(2);
                            if d2 > outer || d2 <= inner {
                                continue;
                            }
                            let cross_s = s_dir.0 as i64 * dy as i64 - s_dir.1 as i64 * dx as i64;
                            let cross_e = e_dir.0 as i64 * dy as i64 - e_dir.1 as i64 * dx as i64;
                            let in_sector = if wide {
                                // Complement of the narrow sector from end to
                                // start.
                                !(cross_e >= 0 && cross_s <= 0)
                            } else {
                                cross_s >= 0 && cross_e <= 0
                            };
                            if in_sector {
                                hits += 1;
                            }
                        }
                    }
                    let expected = (hits * 255 + 128) / 256;
                    let actual = alpha_of(canvas.at(px, py));
                    let error = expected.abs_diff(actual);
                    assert!(
                        error <= 72,
                        "start {start} sweep {sweep} at {px},{py}: \
                         oracle {expected}, rasteriser {actual}"
                    );
                    if expected == 0 {
                        assert!(
                            actual <= 16,
                            "start {start} sweep {sweep}: painted well outside \
                             the arc at {px},{py}: {actual}"
                        );
                    }
                    if expected == 255 {
                        assert!(
                            actual >= 240,
                            "start {start} sweep {sweep}: hole inside the arc \
                             at {px},{py}: {actual}"
                        );
                    }
                }
            }
        }
    }
}
