//! The object an application holds: fonts, a cache, measurement and drawing.

use alloc::boxed::Box;
use alloc::vec::Vec;

use denise::{Color, Point, Rect, Size};
use denise_render::Canvas;

use crate::atlas::{AtlasStats, GlyphAtlas, GlyphKey};
use crate::bitmap::BitmapSource;
use crate::source::{FontId, FontMetrics, GlyphId, GlyphSource, ShapedGlyph};

/// A font and a size, together, because neither is much use alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextStyle {
    /// Which registered font.
    pub font: FontId,
    /// Requested size in pixels. A source may snap it; see
    /// [`GlyphSource::snap_size`].
    pub size_px: u16,
}

impl TextStyle {
    /// The built-in font at `size_px`.
    pub const fn built_in(size_px: u16) -> Self {
        Self {
            font: FontId(0),
            size_px,
        }
    }

    /// The same style at a different size.
    pub const fn with_size(mut self, size_px: u16) -> Self {
        self.size_px = size_px;
        self
    }
}

/// Where a laid-out glyph goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PositionedGlyph {
    /// Which glyph. Not a character — see [`GlyphId`].
    pub glyph: GlyphId,
    /// Pen position before this glyph, relative to the start of the line.
    pub pen_x: i32,
    /// Where the ink goes, relative to the line's origin and baseline.
    pub bounds: Rect,
}

/// Fonts, a bounded glyph cache, and everything that needs both.
///
/// One of these per application. It is `&mut` for measurement as well as drawing,
/// because measuring is what populates the cache: a label measured during layout
/// and drawn a moment later rasterises its glyphs once, and a label measured on
/// every layout pass and never redrawn pays a cache lookup rather than an outline
/// computation each time.
pub struct TextEngine {
    atlas: GlyphAtlas,
    sources: Vec<Box<dyn GlyphSource>>,
    /// Reused across calls so laying out a line allocates nothing after the
    /// first one, which matters because measurement happens every layout pass.
    run: Vec<ShapedGlyph>,
}

impl TextEngine {
    /// An engine with the built-in bitmap font registered as [`FontId(0)`], and a
    /// 64 KB glyph cache.
    ///
    /// `FontId(0)` is always the built-in font, in every configuration, so a
    /// widget that names no font gets something that certainly exists.
    pub fn new() -> Self {
        Self::with_atlas(GlyphAtlas::with_default_size())
    }

    /// As [`TextEngine::new`], with a cache of a chosen size.
    pub fn with_atlas(atlas: GlyphAtlas) -> Self {
        let mut engine = Self {
            atlas,
            sources: Vec::new(),
            run: Vec::new(),
        };
        engine.add_font(Box::new(BitmapSource::new()));
        engine
    }

    /// Registers a font and returns its id.
    pub fn add_font(&mut self, source: Box<dyn GlyphSource>) -> FontId {
        let id = FontId(self.sources.len() as u16);
        self.sources.push(source);
        id
    }

    /// Number of registered fonts.
    #[inline]
    pub fn font_count(&self) -> usize {
        self.sources.len()
    }

    /// Name of a registered font.
    pub fn font_name(&self, font: FontId) -> Option<&str> {
        self.sources.get(font.0 as usize).map(|s| s.name())
    }

    /// The glyph cache.
    #[inline]
    pub const fn atlas(&self) -> &GlyphAtlas {
        &self.atlas
    }

    /// Cache statistics.
    #[inline]
    pub const fn stats(&self) -> AtlasStats {
        self.atlas.stats()
    }

    /// Empties the glyph cache. Needed after nothing; useful in benches.
    pub fn clear_cache(&mut self) {
        self.atlas.clear();
    }

    /// Vertical metrics for a style.
    pub fn metrics(&self, style: TextStyle) -> FontMetrics {
        self.sources
            .get(style.font.0 as usize)
            .map(|s| s.metrics(style.size_px))
            .unwrap_or_default()
    }

    /// The size this style will actually be drawn at.
    pub fn snap_size(&self, style: TextStyle) -> u16 {
        self.sources
            .get(style.font.0 as usize)
            .map(|s| s.snap_size(style.size_px))
            .unwrap_or(style.size_px)
    }

    /// Baseline-to-baseline distance for a style.
    pub fn line_height(&self, style: TextStyle) -> i32 {
        self.metrics(style).line_height()
    }

    /// Lays out one line, calling `f` for each glyph that has ink.
    ///
    /// Returns the total advance. Positions are relative to the line's start, with
    /// `bounds.y` measured from the baseline — so a caller places the line by
    /// translating, and never has to know how the font was measured.
    pub fn layout_line(
        &mut self,
        style: TextStyle,
        text: &str,
        mut f: impl FnMut(PositionedGlyph),
    ) -> i32 {
        let width = self.shape_into_run(style, text);
        for index in 0..self.run.len() {
            let glyph = self.run[index];
            let Some(placed) = self.placed(style, glyph.id) else {
                continue;
            };
            if placed.metrics.is_blank() {
                continue;
            }
            f(PositionedGlyph {
                glyph: glyph.id,
                pen_x: glyph.x,
                bounds: Rect::new(
                    glyph.x + placed.metrics.bearing_x,
                    glyph.y - placed.metrics.bearing_y,
                    placed.metrics.size.width as i32,
                    placed.metrics.size.height as i32,
                ),
            });
        }
        width
    }

    /// Fills `self.run` with the glyphs of `text`, and returns the run's width.
    ///
    /// A source that shapes does its own layout. Everything else is laid out here,
    /// taking each advance from the glyph cache — which is what makes measuring
    /// the same label on every layout pass cost a cache lookup rather than an
    /// outline computation.
    fn shape_into_run(&mut self, style: TextStyle, text: &str) -> i32 {
        self.run.clear();
        let Some(source) = self.sources.get_mut(style.font.0 as usize) else {
            return 0;
        };
        if source.can_shape() {
            return source.shape(text, style.size_px, &mut self.run);
        }

        let mut pen = 0;
        for ch in text.chars() {
            let Some(id) = source.glyph_id(ch).or_else(|| source.fallback_id(ch)) else {
                continue;
            };
            let key = GlyphKey {
                font: style.font,
                size_px: style.size_px,
                glyph: id,
            };
            let Some(placed) = self.atlas.get_or_insert(key, source.as_mut()) else {
                continue;
            };
            self.run.push(ShapedGlyph { id, x: pen, y: 0 });
            pen += placed.metrics.advance;
        }
        pen
    }

    /// The cached placement of one glyph, rasterising it if need be.
    fn placed(&mut self, style: TextStyle, glyph: crate::GlyphId) -> Option<crate::Placed> {
        let source = self.sources.get_mut(style.font.0 as usize)?;
        let key = GlyphKey {
            font: style.font,
            size_px: style.size_px,
            glyph,
        };
        self.atlas.get_or_insert(key, source.as_mut())
    }

    /// Width of one line, ignoring `\n`.
    pub fn measure_line(&mut self, style: TextStyle, text: &str) -> i32 {
        self.layout_line(style, text, |_| {})
    }

    /// Extent of `text`, honouring `\n`.
    ///
    /// The height is `lines * line_height`, not the ink's bounding box: a label
    /// that changes from `Ok` to `Ogg` must not change height, or a form would
    /// reflow every time a reading gained a descender.
    pub fn measure(&mut self, style: TextStyle, text: &str) -> Size {
        let line_height = self.line_height(style);
        let mut widest = 0;
        let mut lines = 0;
        for line in text.split('\n') {
            widest = widest.max(self.measure_line(style, line));
            lines += 1;
        }
        Size::new(widest.max(0) as u32, (lines * line_height).max(0) as u32)
    }

    /// Draws one line with its baseline at `origin`.
    ///
    /// Returns the total advance.
    pub fn draw_line(
        &mut self,
        canvas: &mut Canvas<'_>,
        style: TextStyle,
        origin: Point,
        text: &str,
        color: Color,
    ) -> i32 {
        let width = self.shape_into_run(style, text);
        for index in 0..self.run.len() {
            let glyph = self.run[index];
            let Some(placed) = self.placed(style, glyph.id) else {
                continue;
            };
            if let Some(mask) = self.atlas.mask(&placed) {
                let at = Point::new(
                    origin.x + glyph.x + placed.metrics.bearing_x,
                    origin.y + glyph.y - placed.metrics.bearing_y,
                );
                canvas.blit_mask(at, &mask, color);
            }
        }
        width
    }

    /// Draws `text` with the top-left corner of its first line at `origin`,
    /// honouring `\n`. Returns the extent laid out.
    ///
    /// Top-left rather than baseline, because a widget positions text in a box and
    /// should not have to know where the baseline of a font it did not choose
    /// happens to fall.
    pub fn draw(
        &mut self,
        canvas: &mut Canvas<'_>,
        style: TextStyle,
        origin: Point,
        text: &str,
        color: Color,
    ) -> Size {
        let metrics = self.metrics(style);
        let line_height = metrics.line_height();
        let mut widest = 0;
        let mut lines = 0;
        for line in text.split('\n') {
            let baseline = Point::new(origin.x, origin.y + metrics.ascent + lines * line_height);
            widest = widest.max(self.draw_line(canvas, style, baseline, line, color));
            lines += 1;
        }
        Size::new(widest.max(0) as u32, (lines * line_height).max(0) as u32)
    }
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for TextEngine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextEngine")
            .field("fonts", &self.sources.len())
            .field("atlas", &self.atlas)
            .finish()
    }
}
