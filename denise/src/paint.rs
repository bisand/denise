//! A colour prepared for drawing.
//!
//! Premultiplication is arithmetic on a word, not rasterisation, and every
//! backend needs the same answer from it — so it lives here rather than in any
//! one renderer. Integer throughout, like everything else this crate owns: no
//! `libm` on a `no_std` target, and the same bytes on x86 and ARM.

use crate::color::Color;

/// Mask selecting the two 8-bit lanes at bits 0..8 and 16..24.
pub const LANES: u32 = 0x00FF_00FF;

/// Multiplies two packed 8-bit lanes by `a` (`0..=255`) and divides by 255 with
/// correct rounding.
///
/// The `+ 0x80` bias and the fold-back of the high bits make this exact, not the
/// usual `>> 8` approximation: `mul_lanes(x, 255) == x` and `mul_lanes(x, 0) == 0`.
/// Cheap approximations get those endpoints wrong, and an opaque fill that lands on
/// 254 instead of 255 is visible as banding the moment anything is drawn twice.
#[inline(always)]
pub const fn mul_lanes(x: u32, a: u32) -> u32 {
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
        let premul = scale_premul(self.premul, coverage);
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

/// Scales a premultiplied `0xAARRGGBB` word — alpha lane included — by a
/// coverage of `0..=255`.
#[inline(always)]
pub const fn scale_premul(px: u32, coverage: u32) -> u32 {
    let rb = mul_lanes(px & LANES, coverage);
    let ag = mul_lanes((px >> 8) & LANES, coverage);
    rb | (ag << 8)
}
