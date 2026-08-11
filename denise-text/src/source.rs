//! What a font has to provide, and what it says about a glyph.

use alloc::vec::Vec;

use denise::Size;

/// Identifies a font within one [`TextEngine`](crate::TextEngine).
///
/// Small and `Copy` because it ends up in every glyph cache key, and because M5's
/// C ABI has to carry it across `extern "C"` without a pointer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontId(pub u16);

/// Identifies a glyph *within one font*.
///
/// Not a `char`, and the distinction is the whole reason the shaping tier can
/// exist. A source that maps characters straight to glyphs uses the code point
/// here; a source that shapes uses the font's own glyph index, because after
/// shaping there is no longer a one-to-one correspondence — `fi` may be one
/// glyph, `é` may be two, and an Arabic letter is a different glyph in the middle
/// of a word than at its end. A cache keyed by `char` cannot represent any of
/// that, so this one is not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlyphId(pub u32);

impl GlyphId {
    /// The identity a character-mapped source uses.
    #[inline]
    pub const fn from_char(ch: char) -> Self {
        Self(ch as u32)
    }

    /// The character this came from, for a source that maps them directly.
    #[inline]
    pub const fn as_char(self) -> Option<char> {
        char::from_u32(self.0)
    }
}

/// One glyph, positioned by whatever laid the line out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShapedGlyph {
    /// Which glyph.
    pub id: GlyphId,
    /// Pen position, relative to the start of the run.
    pub x: i32,
    /// Baseline offset, relative to the run's baseline. Non-zero only for a
    /// source that positions marks vertically.
    pub y: i32,
}

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

/// A thing that can lay out, measure and rasterise glyphs.
///
/// [`shape`](GlyphSource::shape) is the layout step and has a default that
/// accumulates per-character advances — correct for every script where a
/// character is a glyph. A backend that can do better overrides it. Keeping
/// shaping *on this trait* rather than beside it is what lets the cache and the
/// draw path be written once: they deal in [`GlyphId`]s and never need to know
/// whether a shaper produced them.
pub trait GlyphSource {
    /// Human-readable name, for logging which font a panel actually loaded.
    fn name(&self) -> &str;

    /// Vertical metrics at `size_px`.
    fn metrics(&self, size_px: u16) -> FontMetrics;

    /// The glyph a character maps to, or `None` if this source has none.
    ///
    /// Only meaningful for text that needs no shaping. Use [`GlyphSource::shape`]
    /// for anything else, and note that a source which shapes may return `None`
    /// here for a character it can nonetheless render in context.
    fn glyph_id(&self, ch: char) -> Option<GlyphId> {
        self.contains(ch).then(|| GlyphId::from_char(ch))
    }

    /// Metrics for one glyph, without rasterising it.
    ///
    /// Used for measurement, which happens far more often than drawing: a label
    /// that has not changed is measured on every layout pass and drawn on none.
    fn glyph_metrics(&mut self, glyph: GlyphId, size_px: u16) -> Option<GlyphMetrics>;

    /// Rasterises one glyph.
    ///
    /// Returning a borrow of the source's own scratch buffer rather than filling a
    /// caller's slice keeps this to one call, and lets a backend that already has
    /// the bitmap hand it over without copying it twice.
    fn rasterise(&mut self, glyph: GlyphId, size_px: u16) -> Option<Rasterised<'_>>;

    /// Turns a string into positioned glyphs, appended to `out`.
    ///
    /// **Only called when [`can_shape`](GlyphSource::can_shape) is `true`.** A
    /// source that maps characters to glyphs one for one does not implement this:
    /// the engine lays those out itself, taking each advance from the glyph cache
    /// so that measuring a label a hundred times costs one rasterisation rather
    /// than a hundred outline computations.
    ///
    /// Returns the run's total advance, which is its width.
    fn shape(&mut self, text: &str, size_px: u16, out: &mut Vec<ShapedGlyph>) -> i32 {
        let _ = (text, size_px, out);
        0
    }

    /// Returns `true` if this source lays out runs itself through
    /// [`shape`](GlyphSource::shape), rather than one glyph per character.
    ///
    /// Worth logging at startup: a panel that needs ligatures and got a source
    /// that cannot provide them looks subtly wrong rather than obviously broken.
    fn can_shape(&self) -> bool {
        false
    }

    /// The glyph to draw for a character this source does not have.
    ///
    /// `None` drops the character silently, which is almost never what anyone
    /// wants — a visible box is a defect somebody will report.
    fn fallback_id(&self, ch: char) -> Option<GlyphId> {
        let _ = ch;
        None
    }

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
