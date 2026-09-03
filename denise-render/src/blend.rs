//! Pixel arithmetic.
//!
//! Everything here is integer. There is no `f32` in the rasteriser at all, which
//! buys three things: no `libm` dependency in a `no_std` build, no FPU traffic on
//! targets where that is expensive, and — most usefully — output that is
//! bit-identical on x86 and ARM, so a golden-image test is a meaningful test.

pub use denise::Paint;
pub use denise::paint::scale_premul;
use denise::paint::{LANES, mul_lanes};


/// Premultiplies straight-alpha `0xAARRGGBB` words in place.
///
/// This is [`Paint::new`]'s arithmetic applied to a buffer: exact at both
/// endpoints, done once at load time so a blit never divides by 255 per frame.
/// Decoders hand out straight alpha; [`Canvas::blit`](crate::Canvas::blit)
/// consumes premultiplied — this is the bridge between them.
pub fn premultiply(pixels: &mut [u32]) {
    for px in pixels {
        match *px >> 24 {
            255 => {}
            0 => *px = 0,
            a => {
                let rb = mul_lanes(*px & LANES, a);
                let g = mul_lanes((*px >> 8) & 0xFF, a);
                *px = (a << 24) | rb | (g << 8);
            }
        }
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
    use denise::Color;

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
