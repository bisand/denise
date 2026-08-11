//! The built-in bitmap font, as a glyph source.
//!
//! Always available, needs no files, no allocator beyond a scratch buffer and no
//! feature flags. This is the font a panel gets if nobody chooses one, and the
//! font it falls back to if the chosen one fails to load — which on a device that
//! boots from flash and mounts a read-only root is not a hypothetical.

use alloc::vec::Vec;

use denise::Size;
use denise_render::font::{self, BitmapFont, Glyph};

use crate::source::{FontMetrics, GlyphMetrics, GlyphSource, Rasterised};

/// The built-in five-by-seven font at whole-number scales.
#[derive(Debug)]
pub struct BitmapSource {
    font: &'static BitmapFont,
    scratch: Vec<u8>,
}

impl Default for BitmapSource {
    fn default() -> Self {
        Self::new()
    }
}

impl BitmapSource {
    /// Wraps the built-in font.
    pub fn new() -> Self {
        Self {
            font: &font::BUILT_IN,
            scratch: Vec::new(),
        }
    }

    /// Whole-number scale for a requested pixel size, at least 1.
    ///
    /// A bitmap font cannot be scaled continuously. Asking for 13 px and silently
    /// getting 16 is how a layout ends up three pixels wrong for reasons nobody
    /// can find, so [`snap_size`](GlyphSource::snap_size) reports what will
    /// actually happen.
    #[inline]
    fn scale(size_px: u16) -> i32 {
        (i32::from(size_px) / font::CELL_HEIGHT).max(1)
    }

    /// Bounding box of a glyph's ink, in unscaled cell coordinates.
    ///
    /// Trimming matters: a full stop is one pixel of ink in a five-by-eight cell,
    /// and caching it untrimmed would spend forty bytes of a bounded atlas on
    /// thirty-nine blank ones.
    fn ink(glyph: &Glyph) -> Option<(i32, i32, i32, i32)> {
        let mut left = font::CELL_WIDTH;
        let mut right = 0;
        let mut top = font::CELL_HEIGHT;
        let mut bottom = 0;
        for (row, bits) in glyph.iter().enumerate() {
            if *bits == 0 {
                continue;
            }
            let row = row as i32;
            top = top.min(row);
            bottom = bottom.max(row + 1);
            for x in 0..font::CELL_WIDTH {
                if bits & (0x80 >> x) != 0 {
                    left = left.min(x);
                    right = right.max(x + 1);
                }
            }
        }
        (right > left && bottom > top).then_some((left, top, right, bottom))
    }

    fn metrics_for(&self, ch: char, size_px: u16) -> GlyphMetrics {
        let scale = Self::scale(size_px);
        let glyph = self.font.glyph(ch);
        // The baseline sits at the bottom of the seven-row body, one row above the
        // bottom of the cell — which is the row descenders use.
        let baseline_row = font::CELL_HEIGHT - 1;
        match Self::ink(glyph) {
            None => GlyphMetrics {
                advance: font::ADVANCE * scale,
                ..GlyphMetrics::default()
            },
            Some((left, top, right, bottom)) => GlyphMetrics {
                advance: font::ADVANCE * scale,
                bearing_x: left * scale,
                bearing_y: (baseline_row - top) * scale,
                size: Size::new(
                    ((right - left) * scale) as u32,
                    ((bottom - top) * scale) as u32,
                ),
            },
        }
    }
}

impl GlyphSource for BitmapSource {
    fn name(&self) -> &str {
        "built-in 5x7"
    }

    fn metrics(&self, size_px: u16) -> FontMetrics {
        let scale = Self::scale(size_px);
        FontMetrics {
            ascent: (font::CELL_HEIGHT - 1) * scale,
            descent: scale,
            line_gap: (font::LINE_HEIGHT - font::CELL_HEIGHT) * scale,
        }
    }

    fn glyph_metrics(&mut self, ch: char, size_px: u16) -> Option<GlyphMetrics> {
        Some(self.metrics_for(ch, size_px))
    }

    fn rasterise(&mut self, ch: char, size_px: u16) -> Option<Rasterised<'_>> {
        let scale = Self::scale(size_px);
        let metrics = self.metrics_for(ch, size_px);
        if metrics.is_blank() {
            self.scratch.clear();
            return Some(Rasterised {
                metrics,
                coverage: &self.scratch,
                stride: 0,
            });
        }

        let glyph = *self.font.glyph(ch);
        let (left, top, _, _) = Self::ink(&glyph).expect("non-blank glyph has ink");
        let width = metrics.size.width as usize;
        let height = metrics.size.height as usize;
        self.scratch.clear();
        self.scratch.resize(width * height, 0);

        // A bitmap font has no partial coverage: every pixel is on or off, which
        // is exactly what makes it cheap to blit and exactly why it looks like
        // what it is at large sizes.
        for y in 0..height {
            let source_row = top + (y as i32 / scale);
            let bits = glyph[source_row as usize];
            if bits == 0 {
                continue;
            }
            let row = &mut self.scratch[y * width..(y + 1) * width];
            for (x, out) in row.iter_mut().enumerate() {
                let source_x = left + (x as i32 / scale);
                if bits & (0x80 >> source_x) != 0 {
                    *out = 255;
                }
            }
        }

        Some(Rasterised {
            metrics,
            coverage: &self.scratch,
            stride: width,
        })
    }

    fn contains(&self, ch: char) -> bool {
        self.font.contains(ch)
    }

    fn snap_size(&self, size_px: u16) -> u16 {
        (Self::scale(size_px) * font::CELL_HEIGHT) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_snap_to_whole_scales() {
        let source = BitmapSource::new();
        assert_eq!(source.snap_size(8), 8);
        assert_eq!(
            source.snap_size(13),
            8,
            "13 px cannot be drawn; 8 is honest"
        );
        assert_eq!(source.snap_size(16), 16);
        assert_eq!(source.snap_size(0), 8, "never smaller than one whole scale");
    }

    #[test]
    fn a_space_has_advance_and_no_ink() {
        let mut source = BitmapSource::new();
        let metrics = source.glyph_metrics(' ', 16).expect("space");
        assert!(metrics.is_blank());
        assert_eq!(metrics.advance, font::ADVANCE * 2);
    }

    #[test]
    fn ink_is_trimmed_to_the_glyph() {
        let mut source = BitmapSource::new();
        // A full stop is one blob in the bottom left of the cell, nothing else.
        let dot = source.glyph_metrics('.', 8).expect("full stop");
        let m = source.glyph_metrics('M', 8).expect("M");
        assert!(
            dot.size.width < m.size.width && dot.size.height < m.size.height,
            "a full stop should not occupy an M-sized cell: {dot:?} vs {m:?}"
        );
        assert!(dot.advance == m.advance, "the font is monospace");
    }

    #[test]
    fn a_descender_reaches_below_the_baseline() {
        let mut source = BitmapSource::new();
        let g = source.glyph_metrics('g', 8).expect("g");
        let o = source.glyph_metrics('o', 8).expect("o");
        // `bearing_y` is measured up from the baseline, so a descender's mask is
        // taller than its bearing and an x-height letter's is not.
        assert!(
            g.size.height as i32 > g.bearing_y,
            "g should hang below the baseline: {g:?}"
        );
        assert!(
            o.size.height as i32 <= o.bearing_y,
            "o should sit on the baseline: {o:?}"
        );
    }

    #[test]
    fn rasterising_fills_exactly_the_declared_extent() {
        let mut source = BitmapSource::new();
        for scale in [1u16, 2, 3] {
            let size = scale * 8;
            let glyph = source.rasterise('M', size).expect("M");
            let m = glyph.metrics;
            assert_eq!(glyph.stride, m.size.width as usize);
            assert_eq!(
                glyph.coverage.len(),
                (m.size.width * m.size.height) as usize,
                "at {size} px"
            );
            assert!(glyph.coverage.contains(&255), "M has ink");
            assert!(
                glyph.coverage.iter().all(|&c| c == 0 || c == 255),
                "a bitmap font has no partial coverage"
            );
        }
    }

    #[test]
    fn scaling_multiplies_every_dimension() {
        let mut source = BitmapSource::new();
        let one = source.glyph_metrics('M', 8).expect("M");
        let three = source.glyph_metrics('M', 24).expect("M");
        assert_eq!(three.size.width, one.size.width * 3);
        assert_eq!(three.size.height, one.size.height * 3);
        assert_eq!(three.advance, one.advance * 3);
        assert_eq!(three.bearing_y, one.bearing_y * 3);
    }

    #[test]
    fn line_height_matches_the_font_module() {
        let source = BitmapSource::new();
        assert_eq!(source.metrics(8).line_height(), font::LINE_HEIGHT);
        assert_eq!(source.metrics(24).line_height(), font::LINE_HEIGHT * 3);
    }

    #[test]
    fn an_unmapped_character_still_rasterises_as_the_missing_box() {
        let mut source = BitmapSource::new();
        assert!(!source.contains('\u{4e2d}'));
        let glyph = source.rasterise('\u{4e2d}', 16).expect("fallback box");
        assert!(!glyph.metrics.is_blank(), "a missing glyph must be visible");
    }
}
