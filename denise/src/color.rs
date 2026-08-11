//! Straight (non-premultiplied) 8-bit-per-channel colour.

/// An sRGB colour with straight alpha.
///
/// Alpha is *not* premultiplied here. Premultiplication happens in the rasteriser,
/// where it can be done once per draw call rather than once per pixel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
    /// Alpha channel. `255` is opaque.
    pub a: u8,
}

impl Color {
    /// Fully transparent.
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
    /// Opaque black.
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    /// Opaque white.
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    /// An opaque colour.
    #[inline]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// A colour with straight alpha.
    #[inline]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Decodes a `0xAARRGGBB` word.
    #[inline]
    pub const fn from_argb8888(v: u32) -> Self {
        Self {
            a: (v >> 24) as u8,
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
        }
    }

    /// Decodes a `0xRRGGBB` word as opaque.
    #[inline]
    pub const fn from_rgb888(v: u32) -> Self {
        Self::from_argb8888(v | 0xFF00_0000)
    }

    /// Encodes to the `0xAARRGGBB` word layout used by [`crate::PixelFormat`].
    #[inline]
    pub const fn to_argb8888(self) -> u32 {
        (self.a as u32) << 24 | (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32
    }

    /// Returns `true` if the colour needs no blending.
    #[inline]
    pub const fn is_opaque(self) -> bool {
        self.a == 255
    }

    /// Returns `true` if the colour contributes nothing.
    #[inline]
    pub const fn is_transparent(self) -> bool {
        self.a == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argb_round_trip() {
        let c = Color::rgba(0x12, 0x34, 0x56, 0x78);
        assert_eq!(c.to_argb8888(), 0x7812_3456);
        assert_eq!(Color::from_argb8888(0x7812_3456), c);
    }

    #[test]
    fn rgb_is_opaque() {
        assert!(Color::from_rgb888(0x1E1E2E).is_opaque());
        assert_eq!(Color::from_rgb888(0x1E1E2E).to_argb8888(), 0xFF1E_1E2E);
    }
}
