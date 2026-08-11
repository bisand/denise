//! Pixel arithmetic.
//!
//! Everything here is integer. There is no `f32` in the rasteriser at all, which
//! buys three things: no `libm` dependency in a `no_std` build, no FPU traffic on
//! targets where that is expensive, and — most usefully — output that is
//! bit-identical on x86 and ARM, so a golden-image test is a meaningful test.

use denise::Color;

/// Mask selecting the two 8-bit lanes at bits 0..8 and 16..24.
const LANES: u32 = 0x00FF_00FF;

/// Multiplies two packed 8-bit lanes by `a` (`0..=255`) and divides by 255 with
/// correct rounding.
///
/// The `+ 0x80` bias and the fold-back of the high bits make this exact, not the
/// usual `>> 8` approximation: `mul_lanes(x, 255) == x` and `mul_lanes(x, 0) == 0`.
/// Cheap approximations get those endpoints wrong, and an opaque fill that lands on
/// 254 instead of 255 is visible as banding the moment anything is drawn twice.
#[inline(always)]
const fn mul_lanes(x: u32, a: u32) -> u32 {
    let t = x * a + 0x0080_0080;
    ((t + ((t >> 8) & LANES)) >> 8) & LANES
}

/// A colour prepared for drawing.
///
/// Premultiplication happens once here rather than once per pixel. Constructing a
/// `Paint` is the only division-by-255 in a fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paint {
    /// `0xAARRGGBB`, colour channels premultiplied by alpha.
    premul: u32,
    /// Straight alpha, `0..=255`.
    alpha: u32,
}

impl Paint {
    /// Prepares a colour for drawing.
    #[inline]
    pub const fn new(color: Color) -> Self {
        let a = color.a as u32;
        let rb = mul_lanes(((color.r as u32) << 16) | color.b as u32, a);
        let g = mul_lanes(color.g as u32, a);
        Self {
            premul: (a << 24) | rb | (g << 8),
            alpha: a,
        }
    }

    /// The premultiplied `0xAARRGGBB` word.
    #[inline]
    pub const fn premultiplied(self) -> u32 {
        self.premul
    }

    /// Straight alpha, `0..=255`.
    #[inline]
    pub const fn alpha(self) -> u32 {
        self.alpha
    }

    /// Returns `true` if drawing can skip the read-modify-write and just store.
    #[inline]
    pub const fn is_opaque(self) -> bool {
        self.alpha == 255
    }

    /// Returns `true` if drawing would change nothing.
    #[inline]
    pub const fn is_invisible(self) -> bool {
        self.alpha == 0
    }

    /// This paint scaled by an anti-aliasing coverage of `0..=255`.
    #[inline]
    pub const fn scaled(self, coverage: u32) -> Self {
        let rb = mul_lanes(self.premul & LANES, coverage);
        let ag = mul_lanes((self.premul >> 8) & LANES, coverage);
        let premul = rb | (ag << 8);
        Self {
            premul,
            alpha: premul >> 24,
        }
    }
}

impl From<Color> for Paint {
    #[inline]
    fn from(color: Color) -> Self {
        Paint::new(color)
    }
}

/// Composites a premultiplied source over a destination pixel.
///
/// Both words are `0xAARRGGBB`. The alpha lane is composited too, so this is
/// correct for an `Argb8888` target as well as an opaque `Xrgb8888` one.
#[inline(always)]
pub const fn source_over(dst: u32, src_premul: u32, alpha: u32) -> u32 {
    let inv = 255 - alpha;
    let rb = mul_lanes(dst & LANES, inv);
    let ag = mul_lanes((dst >> 8) & LANES, inv);
    // No lane can carry: s*a/255 + d*(255-a)/255 <= 255, and neither term can land
    // exactly on .5, so the two roundings cannot both push up.
    src_premul + (rb | (ag << 8))
}

/// Overwrites a span with an opaque word.
#[inline]
pub fn fill_span(span: &mut [u32], word: u32) {
    span.fill(word);
}

/// Composites a constant paint over a span.
#[inline]
pub fn blend_span(span: &mut [u32], paint: Paint) {
    if paint.is_invisible() {
        return;
    }
    if paint.is_opaque() {
        span.fill(paint.premultiplied());
        return;
    }
    let src = paint.premultiplied();
    let alpha = paint.alpha();
    for px in span {
        *px = source_over(*px, src, alpha);
    }
}

/// Composites a paint over a single pixel at `coverage` (`0..=255`).
#[inline]
pub fn blend_pixel(dst: &mut u32, paint: Paint, coverage: u32) {
    if coverage == 0 {
        return;
    }
    let paint = if coverage == 255 {
        paint
    } else {
        paint.scaled(coverage)
    };
    *dst = source_over(*dst, paint.premultiplied(), paint.alpha());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_paint_replaces_destination() {
        let p = Paint::new(Color::rgb(10, 20, 30));
        assert!(p.is_opaque());
        assert_eq!(
            source_over(0xFFFF_FFFF, p.premultiplied(), p.alpha()),
            0xFF0A_141E
        );
    }

    #[test]
    fn transparent_paint_preserves_destination() {
        let p = Paint::new(Color::rgba(10, 20, 30, 0));
        assert_eq!(
            source_over(0xFF12_3456, p.premultiplied(), p.alpha()),
            0xFF12_3456
        );
    }

    #[test]
    fn premultiply_is_exact_at_the_endpoints() {
        // The whole point of the rounding correction: full alpha must round-trip.
        let c = Color::rgba(0xAB, 0xCD, 0xEF, 255);
        assert_eq!(Paint::new(c).premultiplied(), 0xFFAB_CDEF);
        assert_eq!(
            Paint::new(Color::rgba(0xAB, 0xCD, 0xEF, 0)).premultiplied(),
            0
        );
    }

    #[test]
    fn half_alpha_over_black_is_half_the_colour() {
        let p = Paint::new(Color::rgba(200, 100, 50, 128));
        let out = source_over(0xFF00_0000, p.premultiplied(), p.alpha());
        // 200 * 128/255 = 100.4, 100 * 128/255 = 50.2, 50 * 128/255 = 25.1
        assert_eq!(out & 0x00FF_FFFF, 0x0064_3219);
    }

    #[test]
    fn no_lane_ever_carries_into_its_neighbour() {
        // Exhaustive over alpha for the worst-case saturated channels: a carry here
        // would corrupt the neighbouring channel rather than merely round badly.
        for a in 0..=255u32 {
            let p = Paint::new(Color::rgba(255, 255, 255, a as u8));
            let out = source_over(0xFFFF_FFFF, p.premultiplied(), p.alpha());
            assert_eq!(out, 0xFFFF_FFFF, "alpha {a} carried");
        }
    }

    #[test]
    fn blending_is_monotonic_in_alpha() {
        let mut previous = 0u32;
        for a in 0..=255u32 {
            let p = Paint::new(Color::rgba(255, 0, 0, a as u8));
            let red = source_over(0xFF00_0000, p.premultiplied(), p.alpha()) >> 16 & 0xFF;
            assert!(red >= previous, "alpha {a} went backwards");
            previous = red;
        }
        assert_eq!(previous, 255);
    }

    #[test]
    fn coverage_scaling_matches_direct_alpha() {
        // Painting at alpha 255 with coverage c must equal painting at alpha c.
        for c in [0u32, 1, 64, 127, 128, 200, 254, 255] {
            let scaled = Paint::new(Color::rgb(200, 100, 50)).scaled(c);
            let direct = Paint::new(Color::rgba(200, 100, 50, c as u8));
            assert_eq!(scaled.alpha(), direct.alpha(), "coverage {c}");
            assert_eq!(
                scaled.premultiplied(),
                direct.premultiplied(),
                "coverage {c}"
            );
        }
    }
}
