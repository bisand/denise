//! Framebuffer geometry, read from sysfs.
//!
//! The classic way to ask fbdev about itself is `FBIOGET_VSCREENINFO` and
//! `FBIOGET_FSCREENINFO`. Everything those return that matters here is also in
//! `/sys/class/graphics/fbN/`, and reading files instead of issuing ioctls means
//! no `libc` dependency, no `unsafe`, and — the part that actually pays — parsing
//! that is testable on a machine with no framebuffer.
//!
//! What sysfs does not expose is the colour bitfield layout, so the byte order
//! within a pixel is assumed rather than read. See [`PixelLayout`].

use core::fmt;

use denise::Size;

/// How pixels are laid out in the mapped framebuffer.
///
/// sysfs reports the depth but not the channel order, so this is inferred. The
/// assumptions hold for every mainline framebuffer driver and for DRM's fbdev
/// emulation, which is what almost every modern `/dev/fb0` actually is; a device
/// with an exotic byte order will render with its channels swapped rather than
/// fail, and that is the trade a legacy fallback should make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PixelLayout {
    /// 32 bits per pixel, `0xXXRRGGBB`. Matches Denise's own word layout, so
    /// presenting is a copy.
    Xrgb8888,
    /// 16 bits per pixel, 5 red / 6 green / 5 blue. Common on SPI panels, and
    /// needs a conversion on the way out.
    Rgb565,
}

impl PixelLayout {
    /// Bytes each pixel occupies.
    #[inline]
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            PixelLayout::Xrgb8888 => 4,
            PixelLayout::Rgb565 => 2,
        }
    }

    /// Infers the layout from a bit depth.
    pub const fn from_bits_per_pixel(bpp: u32) -> Option<Self> {
        match bpp {
            32 => Some(PixelLayout::Xrgb8888),
            16 => Some(PixelLayout::Rgb565),
            _ => None,
        }
    }

    /// Converts one `0xAARRGGBB` word to this layout's 16-bit encoding.
    ///
    /// Truncation, not dithering: a UI is flat colour and gradients are rare, so
    /// the banding dithering would fix mostly is not there to fix.
    #[inline]
    pub const fn to_rgb565(word: u32) -> u16 {
        let r = (word >> 19) & 0x1F;
        let g = (word >> 10) & 0x3F;
        let b = (word >> 3) & 0x1F;
        ((r << 11) | (g << 5) | b) as u16
    }
}

/// The geometry of a framebuffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FbInfo {
    /// Visible extent in pixels.
    pub size: Size,
    /// Distance between the starts of consecutive rows, in **bytes**.
    ///
    /// fbdev calls this `line_length`, and it is routinely wider than
    /// `width * bytes_per_pixel`. Assuming otherwise is the classic fbdev bug: a
    /// picture that shears diagonally on hardware and looks fine in a VM.
    pub stride_bytes: u32,
    /// Bit depth as reported.
    pub bits_per_pixel: u32,
    /// Inferred pixel layout.
    pub layout: PixelLayout,
}

impl FbInfo {
    /// Builds the geometry from raw sysfs attribute contents.
    ///
    /// `modes` is preferred for the visible extent because `virtual_size` includes
    /// any area reserved for panning, which on a double-buffered framebuffer is
    /// twice the height of what is actually on screen.
    pub fn from_sysfs(
        virtual_size: &str,
        modes: &str,
        stride: &str,
        bits_per_pixel: &str,
    ) -> Result<Self, FbInfoError> {
        let virtual_size = parse_virtual_size(virtual_size)?;
        let size = parse_modes(modes).unwrap_or(virtual_size);

        // A mode wider than the allocation cannot be right; trust the allocation.
        let size = Size::new(
            size.width.min(virtual_size.width),
            size.height.min(virtual_size.height),
        );

        let stride_bytes: u32 = stride.trim().parse().map_err(|_| FbInfoError::Unparsable {
            attribute: "stride",
        })?;

        let bits_per_pixel: u32 =
            bits_per_pixel
                .trim()
                .parse()
                .map_err(|_| FbInfoError::Unparsable {
                    attribute: "bits_per_pixel",
                })?;

        let layout = PixelLayout::from_bits_per_pixel(bits_per_pixel)
            .ok_or(FbInfoError::UnsupportedDepth { bits_per_pixel })?;

        if size.is_empty() {
            return Err(FbInfoError::EmptyGeometry);
        }

        let minimum = size.width as usize * layout.bytes_per_pixel();
        if (stride_bytes as usize) < minimum {
            return Err(FbInfoError::StrideTooNarrow {
                stride_bytes,
                required: minimum as u32,
            });
        }

        Ok(Self {
            size,
            stride_bytes,
            bits_per_pixel,
            layout,
        })
    }

    /// Bytes the mapping must cover for this geometry.
    pub fn required_bytes(&self) -> usize {
        // The last row needs only its visible pixels, not the padding after them.
        self.stride_bytes as usize * (self.size.height as usize - 1)
            + self.size.width as usize * self.layout.bytes_per_pixel()
    }

    /// Row stride in pixels, when that is a whole number.
    pub fn stride_pixels(&self) -> Option<u32> {
        let bpp = self.layout.bytes_per_pixel() as u32;
        self.stride_bytes
            .is_multiple_of(bpp)
            .then(|| self.stride_bytes / bpp)
    }
}

impl fmt::Display for FbInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}x{} {}bpp {:?}, stride {} bytes",
            self.size.width, self.size.height, self.bits_per_pixel, self.layout, self.stride_bytes
        )
    }
}

/// `virtual_size` is `"<width>,<height>"`.
fn parse_virtual_size(text: &str) -> Result<Size, FbInfoError> {
    let (w, h) = text.trim().split_once(',').ok_or(FbInfoError::Unparsable {
        attribute: "virtual_size",
    })?;
    let width = w.trim().parse().map_err(|_| FbInfoError::Unparsable {
        attribute: "virtual_size",
    })?;
    let height = h.trim().parse().map_err(|_| FbInfoError::Unparsable {
        attribute: "virtual_size",
    })?;
    Ok(Size::new(width, height))
}

/// `modes` lists entries like `"U:1280x800p-60"`. Only the extent is wanted.
///
/// Returns `None` rather than failing: the attribute is empty on some drivers and
/// `virtual_size` is a perfectly good fallback.
fn parse_modes(text: &str) -> Option<Size> {
    // "U:1280x800p-60" -> drop the "U:" tag, split on 'x', then take the leading
    // digits of "800p-60" and ignore the timing suffix.
    let line = text.lines().next()?.trim();
    let geometry = line.rsplit_once(':').map_or(line, |(_, rest)| rest);
    let (width, rest) = geometry.split_once('x')?;

    let width: u32 = width.trim().parse().ok()?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let height: u32 = digits.parse().ok()?;

    (width > 0 && height > 0).then(|| Size::new(width, height))
}

/// Why a framebuffer's geometry could not be understood.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FbInfoError {
    /// A sysfs attribute did not hold what it should.
    #[error("could not parse the {attribute} attribute")]
    Unparsable {
        /// Which attribute.
        attribute: &'static str,
    },

    /// The depth is not one this backend can drive.
    #[error("unsupported depth: {bits_per_pixel} bits per pixel")]
    UnsupportedDepth {
        /// The depth reported.
        bits_per_pixel: u32,
    },

    /// The framebuffer reported a zero dimension.
    #[error("the framebuffer has no visible area")]
    EmptyGeometry,

    /// The reported stride cannot hold one row.
    #[error("stride of {stride_bytes} bytes cannot hold a row needing {required}")]
    StrideTooNarrow {
        /// The stride reported.
        stride_bytes: u32,
        /// What one row needs.
        required: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exactly what the Alpine VM's virtio-gpu fbdev emulation reports.
    #[test]
    fn reads_a_real_devices_attributes() {
        let info = FbInfo::from_sysfs("1280,800", "U:1280x800p-0", "5120", "32")
            .expect("a real device should parse");
        assert_eq!(info.size, Size::new(1280, 800));
        assert_eq!(info.stride_bytes, 5120);
        assert_eq!(info.layout, PixelLayout::Xrgb8888);
        assert_eq!(info.stride_pixels(), Some(1280));
        assert_eq!(info.required_bytes(), 1280 * 800 * 4);
    }

    #[test]
    fn padded_stride_is_honoured() {
        // The case that shears on hardware and looks perfect in a VM.
        let info = FbInfo::from_sysfs("1366,768", "U:1366x768p-60", "5504", "32").expect("parses");
        assert_eq!(info.stride_bytes, 5504);
        assert_eq!(info.stride_pixels(), Some(1376));
        assert_ne!(info.stride_pixels(), Some(info.size.width));
    }

    #[test]
    fn visible_size_comes_from_modes_not_the_panning_allocation() {
        // A framebuffer allocated at double height for panning is still 800 tall.
        let info = FbInfo::from_sysfs("1280,1600", "U:1280x800p-60", "5120", "32").expect("parses");
        assert_eq!(info.size, Size::new(1280, 800));
    }

    #[test]
    fn an_empty_modes_attribute_falls_back_to_virtual_size() {
        let info = FbInfo::from_sysfs("800,480", "", "3200", "32").expect("parses");
        assert_eq!(info.size, Size::new(800, 480));
    }

    #[test]
    fn a_mode_larger_than_the_allocation_is_clamped() {
        let info = FbInfo::from_sysfs("800,480", "U:1920x1080p-60", "3200", "32").expect("parses");
        assert_eq!(info.size, Size::new(800, 480));
    }

    #[test]
    fn sixteen_bit_panels_are_supported() {
        let info = FbInfo::from_sysfs("480,320", "U:480x320p-60", "960", "16").expect("parses");
        assert_eq!(info.layout, PixelLayout::Rgb565);
        assert_eq!(info.required_bytes(), 480 * 320 * 2);
    }

    #[test]
    fn odd_depths_are_rejected_rather_than_guessed_at() {
        assert_eq!(
            FbInfo::from_sysfs("640,480", "", "1920", "24"),
            Err(FbInfoError::UnsupportedDepth { bits_per_pixel: 24 })
        );
        assert!(matches!(
            FbInfo::from_sysfs("640,480", "", "640", "8"),
            Err(FbInfoError::UnsupportedDepth { .. })
        ));
    }

    #[test]
    fn a_stride_too_narrow_for_a_row_is_rejected() {
        assert!(matches!(
            FbInfo::from_sysfs("1280,800", "", "2560", "32"),
            Err(FbInfoError::StrideTooNarrow { .. })
        ));
    }

    #[test]
    fn rubbish_attributes_are_reported_by_name() {
        assert_eq!(
            FbInfo::from_sysfs("nonsense", "", "5120", "32"),
            Err(FbInfoError::Unparsable {
                attribute: "virtual_size"
            })
        );
        assert_eq!(
            FbInfo::from_sysfs("1280,800", "", "wide", "32"),
            Err(FbInfoError::Unparsable {
                attribute: "stride"
            })
        );
    }

    #[test]
    fn a_zero_dimension_is_rejected() {
        assert_eq!(
            FbInfo::from_sysfs("1280,0", "", "5120", "32"),
            Err(FbInfoError::EmptyGeometry)
        );
    }

    #[test]
    fn rgb565_conversion_keeps_the_extremes_exact() {
        assert_eq!(PixelLayout::to_rgb565(0xFF00_0000), 0x0000);
        assert_eq!(PixelLayout::to_rgb565(0xFFFF_FFFF), 0xFFFF);
        assert_eq!(PixelLayout::to_rgb565(0xFFFF_0000), 0xF800);
        assert_eq!(PixelLayout::to_rgb565(0xFF00_FF00), 0x07E0);
        assert_eq!(PixelLayout::to_rgb565(0xFF00_00FF), 0x001F);
    }

    #[test]
    fn rgb565_conversion_is_monotonic() {
        // A ramp must never go backwards, or gradients develop bands that move.
        let mut previous = 0u16;
        for level in 0..=255u32 {
            let word = 0xFF00_0000 | (level << 8);
            let green = PixelLayout::to_rgb565(word) & 0x07E0;
            assert!(green >= previous, "green went backwards at {level}");
            previous = green;
        }
    }
}
