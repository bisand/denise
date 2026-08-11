//! What a font has to provide, and what it says about a glyph.

use denise::Size;

/// Identifies a font within one [`TextEngine`](crate::TextEngine).
///
/// Small and `Copy` because it ends up in every glyph cache key, and because M5's
/// C ABI has to carry it across `extern "C"` without a pointer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontId(pub u16);

/// Vertical metrics of a font at one size, in pixels.
///
/// Ascent and descent are both positive distances *from* the baseline, which is
/// the convention that stops the sign of `descent` being a coin toss.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FontMetrics {
    /// How far the tallest glyph rises above the baseline.
    pub ascent: i32,
    /// How far the deepest glyph falls below it.
    pub descent: i32,
    /// Extra space the designer asked for between lines.
    pub line_gap: i32,
}

impl FontMetrics {
    /// Baseline-to-baseline distance.
    #[inline]
    pub const fn line_height(&self) -> i32 {
        self.ascent + self.descent + self.line_gap
    }
}

/// Where one glyph sits relative to the pen, in pixels.
///
/// Following FreeType: `bearing_x` is rightwards from the pen to the mask's left
/// edge and `bearing_y` is **upwards** from the baseline to its top edge. So a
/// glyph is drawn at `(pen.x + bearing_x, baseline - bearing_y)`, and a descender
/// is the case where `bearing_y` is smaller than the mask is tall.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlyphMetrics {
    /// How far the pen moves after this glyph.
    pub advance: i32,
    /// Rightwards from the pen to the mask's left edge.
    pub bearing_x: i32,
    /// Upwards from the baseline to the mask's top edge.
    pub bearing_y: i32,
    /// Extent of the coverage mask. Zero for a glyph with no ink, such as a space.
    pub size: Size,
}

impl GlyphMetrics {
    /// Returns `true` if the glyph has no coverage to draw.
    #[inline]
    pub const fn is_blank(&self) -> bool {
        self.size.width == 0 || self.size.height == 0
    }
}

/// One rasterised glyph, borrowed from whatever scratch space the source used.
#[derive(Clone, Copy, Debug)]
pub struct Rasterised<'a> {
    /// Where it sits and how far the pen moves.
    pub metrics: GlyphMetrics,
    /// Coverage, `0` transparent to `255` solid, row-major.
    pub coverage: &'a [u8],
    /// Bytes per row of `coverage`, at least `metrics.size.width`.
    pub stride: usize,
}

/// A thing that can measure and rasterise glyphs.
///
/// Deliberately per-character rather than per-string: shaping — the step that
/// turns a string into a glyph sequence — belongs to whichever backend can do it,
/// and pretending a bitmap font can do it would be a lie in the type system. A
/// source that shapes exposes that through its own API; this trait is the common
/// denominator every backend really does provide.
pub trait GlyphSource {
    /// Human-readable name, for logging which font a panel actually loaded.
    fn name(&self) -> &str;

    /// Vertical metrics at `size_px`.
    fn metrics(&self, size_px: u16) -> FontMetrics;

    /// Metrics for one glyph, without rasterising it.
    ///
    /// Used for measurement, which happens far more often than drawing: a label
    /// that has not changed is measured on every layout pass and drawn on none.
    fn glyph_metrics(&mut self, ch: char, size_px: u16) -> Option<GlyphMetrics>;

    /// Rasterises one glyph.
    ///
    /// Returning a borrow of the source's own scratch buffer rather than filling a
    /// caller's slice keeps this to one call, and lets a backend that already has
    /// the bitmap hand it over without copying it twice.
    fn rasterise(&mut self, ch: char, size_px: u16) -> Option<Rasterised<'_>>;

    /// Returns `true` if this source has a glyph of its own for `ch`, as opposed
    /// to a fallback box.
    fn contains(&self, ch: char) -> bool;

    /// Sizes this source can actually produce, or `None` if it is continuous.
    ///
    /// A bitmap font can only be scaled by whole numbers; asking it for 13 px and
    /// silently getting 16 is the sort of thing that makes a layout wrong by three
    /// pixels for reasons nobody can find. This makes the snapping visible.
    fn snap_size(&self, size_px: u16) -> u16 {
        size_px
    }
}
