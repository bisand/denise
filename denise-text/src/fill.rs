//! Turns a glyph's outline into a coverage mask.
//!
//! The font reader hands over curves; the atlas wants a byte of coverage per
//! pixel. In between is the signed-area accumulation that font-rs described
//! and most small rasterisers since have used: every edge adds, to the pixels
//! it crosses, the area to its right, signed by whether it runs up or down, and
//! a running sum along each row turns that into how much of each pixel is
//! inside. One pass over the edges and one over the pixels, exact for straight
//! edges, no sorting and no sampling.
//!
//! Overlapping contours — which a variable face is full of — sum to more than
//! one and are clamped, which is the non-zero winding rule a font asks for.
//!
//! This used to be `ab_glyph_rasterizer`. It is a page of arithmetic, and a page
//! of arithmetic is not worth a crate.

use alloc::vec::Vec;

/// How far, in pixels, a flattened curve may stray from the real one. An edge
/// moved a fiftieth of a pixel changes the pixel it crosses by five levels of
/// coverage at the very worst, and over 770 real glyphs at seven sizes the
/// worst was four and the mean a fifth of one. A twentieth was measurably
/// worse — twelve — and a hundredth buys two levels for a quarter more edges.
const TOLERANCE: f32 = 0.02;

/// No curve in a glyph needs this many pieces; one in a broken face might ask.
const MOST_PIECES: u32 = 64;

/// The largest whole number not above `value`: `f32::floor`, which `core` does
/// not have. Glyph coordinates are a few thousand at most, far inside `i32`.
fn floor(value: f32) -> f32 {
    let whole = value as i32 as f32;
    if whole > value { whole - 1.0 } else { whole }
}

/// The smallest whole number not below `value`.
fn ceil(value: f32) -> f32 {
    -floor(-value)
}

/// The smallest count whose square reaches `target`: a square root rounded up,
/// by counting, because `core` has no `sqrt` and the answer is nearly always
/// under ten.
fn pieces(target: f32) -> u32 {
    let mut count = 1;
    while count < MOST_PIECES && ((count * count) as f32) < target {
        count += 1;
    }
    count
}

/// The pixels an outline touches: `x` and `y` of the top left, then the size.
/// `y` grows downwards from the baseline, as the mask's rows do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Bounds {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// An outline, flattened to straight edges as it arrives.
///
/// Coordinates come in as a font gives them, in pixels with `y` growing
/// *upwards* from the baseline, and are kept with `y` growing downwards.
#[derive(Debug, Default)]
pub(crate) struct Outline {
    /// `[x0, y0, x1, y1]` for every edge, closing edges included.
    edges: Vec<[f32; 4]>,
    start: (f32, f32),
    at: (f32, f32),
    /// Accumulated signed area, a row of `width + 2` per row of pixels: an edge
    /// on the right-hand boundary writes one past the last pixel, and the spare
    /// cells keep that out of the row below.
    area: Vec<f32>,
}

impl Outline {
    /// Forgets the last glyph and keeps its allocations.
    pub(crate) fn clear(&mut self) {
        self.edges.clear();
        self.start = (0.0, 0.0);
        self.at = (0.0, 0.0);
    }

    pub(crate) fn move_to(&mut self, x: f32, y: f32) {
        self.close();
        self.start = (x, -y);
        self.at = self.start;
    }

    pub(crate) fn line_to(&mut self, x: f32, y: f32) {
        self.edge_to(x, -y);
    }

    pub(crate) fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let (p0, p1, p2) = (self.at, (cx, -cy), (x, -y));
        // A quadratic's chord error over `n` equal pieces is |p0 - 2p1 + p2| / 4n².
        let bend = (p0.0 - 2.0 * p1.0 + p2.0).abs() + (p0.1 - 2.0 * p1.1 + p2.1).abs();
        let count = pieces(bend / (4.0 * TOLERANCE));
        for step in 1..count {
            let t = step as f32 / count as f32;
            let u = 1.0 - t;
            let (a, b, c) = (u * u, 2.0 * u * t, t * t);
            self.edge_to(
                a * p0.0 + b * p1.0 + c * p2.0,
                a * p0.1 + b * p1.1 + c * p2.1,
            );
        }
        self.edge_to(p2.0, p2.1);
    }

    #[allow(clippy::too_many_arguments, reason = "the six a cubic has")]
    pub(crate) fn curve_to(&mut self, c0x: f32, c0y: f32, c1x: f32, c1y: f32, x: f32, y: f32) {
        let (p0, p1, p2, p3) = (self.at, (c0x, -c0y), (c1x, -c1y), (x, -y));
        // A cubic's is at most 3/4 of its larger second difference, over n².
        let first = (p0.0 - 2.0 * p1.0 + p2.0).abs() + (p0.1 - 2.0 * p1.1 + p2.1).abs();
        let second = (p1.0 - 2.0 * p2.0 + p3.0).abs() + (p1.1 - 2.0 * p2.1 + p3.1).abs();
        let count = pieces(0.75 * first.max(second) / TOLERANCE);
        for step in 1..count {
            let t = step as f32 / count as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.edge_to(
                a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
                a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
            );
        }
        self.edge_to(p3.0, p3.1);
    }

    /// Joins the contour back to where it began. A font's contours are closed
    /// whether or not they say so, and an open one would leak ink along its row.
    pub(crate) fn close(&mut self) {
        let (x, y) = self.start;
        self.edge_to(x, y);
    }

    fn edge_to(&mut self, x: f32, y: f32) {
        // A face that is broken enough can produce these, and one of them in the
        // bounds would size the mask from nonsense.
        if !(x.is_finite() && y.is_finite()) {
            return;
        }
        if (x, y) != self.at {
            self.edges.push([self.at.0, self.at.1, x, y]);
            self.at = (x, y);
        }
    }

    /// The pixels the outline touches, or `None` for one with no ink: a space.
    pub(crate) fn bounds(&self) -> Option<Bounds> {
        let first = self.edges.first()?;
        let (mut left, mut top, mut right, mut bottom) = (first[0], first[1], first[0], first[1]);
        for &[x0, y0, x1, y1] in &self.edges {
            left = left.min(x0).min(x1);
            right = right.max(x0).max(x1);
            top = top.min(y0).min(y1);
            bottom = bottom.max(y0).max(y1);
        }
        let (x, y) = (floor(left), floor(top));
        let (width, height) = (ceil(right) - x, ceil(bottom) - y);
        // Bigger than any glyph, and small enough that the mask is an allocation
        // rather than a way to take the panel down with a crafted face.
        const MOST: f32 = 4096.0;
        if !(width >= 1.0 && height >= 1.0 && width <= MOST && height <= MOST) {
            return None;
        }
        Some(Bounds {
            x: x as i32,
            y: y as i32,
            width: width as u32,
            height: height as u32,
        })
    }

    /// Writes the outline's coverage into `mask`, a row of `bounds.width` bytes
    /// per row of pixels, resized to fit. `bounds` is what [`Outline::bounds`]
    /// said.
    pub(crate) fn fill(&mut self, bounds: Bounds, mask: &mut Vec<u8>) {
        let (width, height) = (bounds.width as usize, bounds.height as usize);
        let stride = width + 2;
        self.area.clear();
        self.area.resize(stride * height, 0.0);
        let (dx, dy) = (bounds.x as f32, bounds.y as f32);
        for &[x0, y0, x1, y1] in &self.edges {
            accumulate(
                &mut self.area,
                stride,
                height,
                (x0 - dx, y0 - dy),
                (x1 - dx, y1 - dy),
            );
        }
        mask.clear();
        mask.reserve(width * height);
        for row in self.area.chunks_exact(stride) {
            let mut sum = 0.0;
            mask.extend(row[..width].iter().map(|area| {
                sum += area;
                (sum.abs().min(1.0) * 255.0 + 0.5) as u8
            }));
        }
    }
}

/// Adds one edge's signed area to every pixel it crosses.
///
/// Both points are inside the mask, its far edges included: the bounds were
/// taken from these same edges. The indexing is checked anyway, so that if
/// that is ever untrue the cost is a glyph drawn wrong rather than a panic.
fn accumulate(area: &mut [f32], stride: usize, height: usize, from: (f32, f32), to: (f32, f32)) {
    if from.1 == to.1 {
        return;
    }
    let (sign, (x_top, y_top), (x_bottom, y_bottom)) = if from.1 < to.1 {
        (1.0, from, to)
    } else {
        (-1.0, to, from)
    };
    let slope = (x_bottom - x_top) / (y_bottom - y_top);
    let right = (stride - 2) as f32;
    let mut x = x_top.clamp(0.0, right);
    let first = y_top.max(0.0) as usize;
    let last = (ceil(y_bottom).max(0.0) as usize).min(height);
    for row in first..last {
        let cells = &mut area[row * stride..(row + 1) * stride];
        // The part of the edge inside this row, and where it leaves the row.
        let dy = (y_bottom.min(row as f32 + 1.0) - y_top.max(row as f32)).max(0.0);
        let x_next = (x + slope * dy).clamp(0.0, right);
        let d = dy * sign;
        // Stepping along the slope drifts by a few parts in a million, and an
        // edge that ends exactly on the mask's boundary arrives a hair outside
        // it. Rounded down that is the pixel before the first, and its area
        // would land one pixel to the right of where it belongs. No edge is
        // really outside, so pulling it back in is exact rather than a fudge.
        let (x0, x1) = if x < x_next { (x, x_next) } else { (x_next, x) };
        let (x0_floor, x1_ceil) = (floor(x0), ceil(x1));
        let (x0_cell, x1_cell) = (x0_floor.max(0.0) as usize, x1_ceil.max(0.0) as usize);
        let mut add = |cell: usize, value: f32| {
            if let Some(cell) = cells.get_mut(cell) {
                *cell += value;
            }
        };
        if x1_cell <= x0_cell + 1 {
            // Within one pixel: it keeps what is to the right of the edge's
            // middle, and the next pixel gets the rest.
            let middle = 0.5 * (x + x_next) - x0_floor;
            add(x0_cell, d - d * middle);
            add(x0_cell + 1, d * middle);
        } else {
            // Across several: a triangle in the first, a trapezium's worth in
            // each one between, and what is left in the last two.
            let per_cell = 1.0 / (x1 - x0);
            let x0_part = x0 - x0_floor;
            let first_area = 0.5 * per_cell * (1.0 - x0_part) * (1.0 - x0_part);
            let x1_part = x1 - x1_ceil + 1.0;
            let last_area = 0.5 * per_cell * x1_part * x1_part;
            add(x0_cell, d * first_area);
            if x1_cell == x0_cell + 2 {
                add(x0_cell + 1, d * (1.0 - first_area - last_area));
            } else {
                let second_area = per_cell * (1.5 - x0_part);
                add(x0_cell + 1, d * (second_area - first_area));
                for cell in x0_cell + 2..x1_cell - 1 {
                    add(cell, d * per_cell);
                }
                let so_far = second_area + (x1_cell - x0_cell - 3) as f32 * per_cell;
                add(x1_cell - 1, d * (1.0 - so_far - last_area));
            }
            add(x1_cell, d * last_area);
        }
        x = x_next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(outline: &mut Outline, left: f32, bottom: f32, right: f32, top: f32) {
        outline.move_to(left, bottom);
        outline.line_to(left, top);
        outline.line_to(right, top);
        outline.line_to(right, bottom);
        outline.close();
    }

    fn filled(outline: &mut Outline) -> (Bounds, Vec<u8>) {
        let bounds = outline.bounds().expect("ink");
        let mut mask = Vec::new();
        outline.fill(bounds, &mut mask);
        assert_eq!(mask.len(), (bounds.width * bounds.height) as usize);
        (bounds, mask)
    }

    #[test]
    fn rounding_goes_the_right_way_on_both_sides_of_zero() {
        assert_eq!(
            (floor(1.5), floor(-1.5), floor(2.0), floor(-2.0)),
            (1.0, -2.0, 2.0, -2.0)
        );
        assert_eq!(
            (ceil(1.5), ceil(-1.5), ceil(2.0), ceil(-2.0)),
            (2.0, -1.0, 2.0, -2.0)
        );
        assert_eq!(
            (pieces(0.0), pieces(1.0), pieces(1.1), pieces(9.0)),
            (1, 1, 2, 3)
        );
        assert_eq!(pieces(f32::INFINITY), MOST_PIECES);
    }

    #[test]
    fn a_box_on_the_pixel_grid_is_solid_and_exactly_its_size() {
        let mut outline = Outline::default();
        rectangle(&mut outline, 1.0, 2.0, 4.0, 7.0);
        let (bounds, mask) = filled(&mut outline);
        // Two to seven above the baseline is rows -7 to -2 going down.
        assert_eq!(
            bounds,
            Bounds {
                x: 1,
                y: -7,
                width: 3,
                height: 5
            }
        );
        assert!(mask.iter().all(|&ink| ink == 255), "{mask:?}");
    }

    #[test]
    fn a_box_wound_the_other_way_is_just_as_solid() {
        let mut outline = Outline::default();
        outline.move_to(0.0, 0.0);
        outline.line_to(3.0, 0.0);
        outline.line_to(3.0, 3.0);
        outline.line_to(0.0, 3.0);
        outline.close();
        let (_, mask) = filled(&mut outline);
        assert!(mask.iter().all(|&ink| ink == 255), "{mask:?}");
    }

    #[test]
    fn a_box_half_way_across_a_pixel_covers_half_of_it() {
        let mut outline = Outline::default();
        rectangle(&mut outline, 0.5, 0.0, 2.5, 1.0);
        let (bounds, mask) = filled(&mut outline);
        assert_eq!((bounds.x, bounds.width, bounds.height), (0, 3, 1));
        assert_eq!(mask, [128, 255, 128]);
    }

    #[test]
    fn a_triangle_covers_half_its_box() {
        let mut outline = Outline::default();
        outline.move_to(0.0, 0.0);
        outline.line_to(16.0, 0.0);
        outline.line_to(0.0, 16.0);
        outline.close();
        let (_, mask) = filled(&mut outline);
        let ink: u32 = mask.iter().map(|&ink| u32::from(ink)).sum();
        let half = 16 * 16 * 255 / 2;
        assert!(ink.abs_diff(half) < half / 100, "{ink} against {half}");
        // The diagonal runs through the corner pixels' middles.
        assert_eq!(mask[0], 128, "top left");
        assert_eq!(mask[15], 0, "top right is outside");
        assert_eq!(mask[15 * 16], 255, "bottom left is inside");
    }

    #[test]
    fn an_edge_that_ends_on_the_boundary_stays_in_the_first_pixel() {
        // A slash: its left edge comes down a long slope to land exactly on the
        // mask's left side, and arrives there a rounding error early. The sliver
        // of ink in the bottom row belongs to the first pixel, not the second.
        let mut outline = Outline::default();
        outline.move_to(0.0, 0.3);
        outline.line_to(6.1, 21.7);
        outline.line_to(7.9, 21.7);
        outline.line_to(1.8, 0.3);
        outline.close();
        let (bounds, mask) = filled(&mut outline);
        assert_eq!((bounds.x, bounds.width, bounds.height), (0, 8, 22));
        let bottom = &mask[21 * 8..];
        assert!(bottom[0] > 100 && bottom[0] < 180, "{bottom:?}");
        for (index, row) in mask.chunks(8).enumerate() {
            let ink: u32 = row.iter().map(|&ink| u32::from(ink)).sum();
            let full = index > 0 && index < 21;
            // A stroke 1.8 wide is 1.8 pixels of ink in every full row.
            assert!(!full || ink.abs_diff(459) <= 3, "row {index}: {ink}");
        }
    }

    #[test]
    fn a_hole_is_empty_and_an_overlap_is_not_darker_than_solid() {
        // A counter, as in an `o`: the inner contour runs the other way.
        let mut outline = Outline::default();
        rectangle(&mut outline, 0.0, 0.0, 6.0, 6.0);
        outline.move_to(2.0, 2.0);
        outline.line_to(4.0, 2.0);
        outline.line_to(4.0, 4.0);
        outline.line_to(2.0, 4.0);
        outline.close();
        let (_, mask) = filled(&mut outline);
        assert_eq!(mask[2 * 6 + 2], 0, "inside the counter");
        assert_eq!(mask[0], 255);

        // Two strokes crossing, as a variable face draws a `+`: same direction.
        let mut outline = Outline::default();
        rectangle(&mut outline, 0.0, 2.0, 6.0, 4.0);
        rectangle(&mut outline, 2.0, 0.0, 4.0, 6.0);
        let (_, mask) = filled(&mut outline);
        assert_eq!(mask[2 * 6 + 2], 255, "where they cross");
        assert_eq!(mask[0], 0, "the corner neither covers");
    }

    #[test]
    fn a_circle_of_curves_has_the_area_of_a_circle() {
        // Four cubics, the usual 0.5523 handles, radius ten about (10, 10).
        const K: f32 = 5.523;
        let mut outline = Outline::default();
        outline.move_to(20.0, 10.0);
        outline.curve_to(20.0, 10.0 + K, 10.0 + K, 20.0, 10.0, 20.0);
        outline.curve_to(10.0 - K, 20.0, 0.0, 10.0 + K, 0.0, 10.0);
        outline.curve_to(0.0, 10.0 - K, 10.0 - K, 0.0, 10.0, 0.0);
        outline.curve_to(10.0 + K, 0.0, 20.0, 10.0 - K, 20.0, 10.0);
        outline.close();
        let (bounds, mask) = filled(&mut outline);
        assert_eq!((bounds.width, bounds.height), (20, 20));
        let ink: f32 = mask.iter().map(|&ink| f32::from(ink) / 255.0).sum();
        let circle = core::f32::consts::PI * 100.0;
        assert!(
            (ink - circle).abs() < circle * 0.005,
            "{ink} against {circle}"
        );
        assert_eq!(mask[10 * 20 + 10], 255, "the middle");
        assert_eq!(mask[0], 0, "the corner");
    }

    #[test]
    fn a_quadratic_lands_where_a_cubic_of_the_same_shape_does() {
        let mut quad = Outline::default();
        quad.move_to(0.0, 0.0);
        quad.quad_to(12.0, 24.0, 24.0, 0.0);
        quad.close();
        let mut cubic = Outline::default();
        cubic.move_to(0.0, 0.0);
        cubic.curve_to(8.0, 16.0, 16.0, 16.0, 24.0, 0.0);
        cubic.close();
        let ((a, quad), (b, cubic)) = (filled(&mut quad), filled(&mut cubic));
        assert_eq!(a, b);
        for (index, (&q, &c)) in quad.iter().zip(&cubic).enumerate() {
            assert!(q.abs_diff(c) <= 2, "pixel {index}: {q} against {c}");
        }
    }

    #[test]
    fn nothing_and_nonsense_have_no_ink() {
        let mut outline = Outline::default();
        assert_eq!(outline.bounds(), None);
        outline.move_to(1.0, 1.0);
        outline.close();
        assert_eq!(outline.bounds(), None, "a contour of one point");
        outline.line_to(f32::NAN, 3.0);
        outline.line_to(f32::INFINITY, 3.0);
        assert_eq!(outline.bounds(), None, "edges to nowhere are dropped");
        outline.line_to(1.0e9, 1.0e9);
        assert_eq!(
            outline.bounds(),
            None,
            "and a mask the size of a wall is refused"
        );
    }

    #[test]
    fn clearing_forgets_the_last_glyph() {
        let mut outline = Outline::default();
        rectangle(&mut outline, 0.0, 0.0, 9.0, 9.0);
        filled(&mut outline);
        outline.clear();
        rectangle(&mut outline, 0.0, 0.0, 2.0, 2.0);
        let (bounds, mask) = filled(&mut outline);
        assert_eq!((bounds.width, bounds.height), (2, 2));
        assert_eq!(mask, [255; 4]);
    }
}
