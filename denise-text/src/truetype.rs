//! Real TrueType and OpenType fonts, behind the `truetype` feature.
//!
//! Anti-aliased, proportionally spaced, at any size — everything the built-in
//! bitmap font cannot do, for about 145 KB of static binary. What it still does
//! not do is *shaping*: no ligatures, no contextual forms, no reordering, no
//! bidirectional text. For Latin, Cyrillic and Greek that costs nothing anyone
//! will notice. For Arabic or Devanagari it is the whole ball game, and those need
//! the `shaping` feature and its three megabytes.
//!
//! # Where the font comes from
//!
//! You supply the bytes. There is no font discovery, no fontconfig and no system
//! font directory, because a device that boots from flash with a read-only root
//! very often has no fonts installed at all, and a UI that renders nothing on a
//! customer's hardware because a directory was empty is not a UI.
//!
//! ```ignore
//! static INTER: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");
//! let source = TrueTypeSource::from_bytes("Inter", INTER)?;
//! let inter = engine.add_font(Box::new(source));
//! ```
//!
//! Embedding with `include_bytes!` is the deployment this is designed for: the
//! font is in the binary, so there is exactly one file to copy to the device and
//! no way for it to arrive without its font.

use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::vec::Vec;

use denise::Size;
use fontdue::{Font, FontSettings};

use crate::source::{FontMetrics, GlyphMetrics, GlyphSource, Rasterised};

/// A parsed TrueType or OpenType face.
pub struct TrueTypeSource {
    name: String,
    font: Font,
    scratch: Vec<u8>,
}

impl TrueTypeSource {
    /// Parses a font from its bytes.
    ///
    /// Returns the parser's complaint on failure, so a panel can log *why* its
    /// font did not load and fall back to the built-in one rather than showing
    /// nothing.
    pub fn from_bytes(name: &str, data: &[u8]) -> Result<Self, String> {
        let font = Font::from_bytes(data, FontSettings::default()).map_err(|e| e.to_owned())?;
        Ok(Self {
            name: name.to_owned(),
            font,
            scratch: Vec::new(),
        })
    }

    /// The parsed face, for callers that need something this trait does not expose.
    #[inline]
    pub const fn font(&self) -> &Font {
        &self.font
    }

    fn convert(metrics: &fontdue::Metrics) -> GlyphMetrics {
        GlyphMetrics {
            // Advances are fractional and pixels are not. Rounding here rather
            // than accumulating in floats keeps a line's width reproducible and
            // keeps the rasteriser's no-floating-point promise intact everywhere
            // except the one crate that has to break it.
            advance: metrics.advance_width.round() as i32,
            bearing_x: metrics.xmin,
            // fontdue reports `ymin` as the bottom of the bitmap relative to the
            // baseline, positive upwards. This trait wants the top.
            bearing_y: metrics.ymin + metrics.height as i32,
            size: Size::new(metrics.width as u32, metrics.height as u32),
        }
    }
}

impl GlyphSource for TrueTypeSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn metrics(&self, size_px: u16) -> FontMetrics {
        match self.font.horizontal_line_metrics(f32::from(size_px)) {
            Some(line) => FontMetrics {
                ascent: line.ascent.round() as i32,
                // fontdue's descent is negative going down; this trait wants a
                // positive distance from the baseline.
                descent: (-line.descent).round() as i32,
                line_gap: line.line_gap.round() as i32,
            },
            // A face with no horizontal metrics is broken, but guessing from the
            // requested size beats returning zeroes and stacking every line on
            // top of the last.
            None => FontMetrics {
                ascent: i32::from(size_px) * 4 / 5,
                descent: i32::from(size_px) / 5,
                line_gap: i32::from(size_px) / 8,
            },
        }
    }

    fn glyph_metrics(&mut self, ch: char, size_px: u16) -> Option<GlyphMetrics> {
        Some(Self::convert(&self.font.metrics(ch, f32::from(size_px))))
    }

    fn rasterise(&mut self, ch: char, size_px: u16) -> Option<Rasterised<'_>> {
        let (metrics, coverage) = self.font.rasterize(ch, f32::from(size_px));
        let converted = Self::convert(&metrics);
        self.scratch.clear();
        self.scratch.extend_from_slice(&coverage);
        Some(Rasterised {
            metrics: converted,
            coverage: &self.scratch,
            stride: metrics.width,
        })
    }

    fn contains(&self, ch: char) -> bool {
        self.font.lookup_glyph_index(ch) != 0
    }
}

impl core::fmt::Debug for TrueTypeSource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TrueTypeSource")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_is_not_a_font_fails_with_a_reason() {
        let error = TrueTypeSource::from_bytes("junk", b"not a font at all")
            .expect_err("that is not a font");
        assert!(!error.is_empty(), "the failure has to say something");
    }

    #[test]
    fn fontdues_ymin_is_converted_to_a_top_bearing() {
        // A 10 px tall bitmap sitting 3 px below the baseline has its top 7 px
        // above it. Getting this sign wrong puts every descender in the wrong
        // place, and only descenders, which is exactly the bug that survives a
        // casual look at a screenshot.
        let metrics = fontdue::Metrics {
            xmin: 1,
            ymin: -3,
            width: 5,
            height: 10,
            advance_width: 6.4,
            advance_height: 0.0,
            bounds: fontdue::OutlineBounds::default(),
        };
        let converted = TrueTypeSource::convert(&metrics);
        assert_eq!(converted.bearing_y, 7);
        assert_eq!(converted.bearing_x, 1);
        assert_eq!(converted.advance, 6, "6.4 rounds down");
        assert_eq!(converted.size, Size::new(5, 10));
    }
}
