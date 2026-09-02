//! The object an application holds: fonts, a cache, measurement and drawing.

use alloc::boxed::Box;
use alloc::vec::Vec;

use denise::{Color, Point, Rect, Size};
use denise_render::Pen;

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
    /// This style at `scale`, never rounding to nothing.
    ///
    /// The text half of the one-multiply DPI pattern: the application scales
    /// its theme, its rectangles and its text styles in the same place. Rounds
    /// to nearest; a text size never scales below one pixel.
    pub fn scaled(self, scale: f32) -> Self {
        let size = (self.size_px as f32 * scale + 0.5) as u16;
        Self {
            size_px: if size == 0 { 1 } else { size },
            ..self
        }
    }

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
    /// Which registered face [`FontId::DEFAULT`] stands for.
    ///
    /// The whole of the default-face mechanism: one indirection, applied where a
    /// style is turned into glyphs, so no widget and no `TextStyle` has to know
    /// about it. See [`TextEngine::set_default_font`].
    default: FontId,
    /// Reused across calls so laying out a line allocates nothing after the
    /// first one, which matters because measurement happens every layout pass.
    run: Vec<ShapedGlyph>,
}

/// Each word in `line`, with its byte offset.
///
/// Runs of spaces collapse: a double space is not an empty word, because a line
/// beginning with a space is a line indented by a typo.
fn word_starts(line: &str) -> impl Iterator<Item = (usize, &str)> {
    line.split(' ')
        .scan(0usize, |offset, word| {
            let start = *offset;
            // The separator is one byte, and it is ASCII, so this stays on a
            // character boundary however many of them are multi-byte.
            *offset += word.len() + 1;
            Some((start, word))
        })
        .filter(|(_, word)| !word.is_empty())
}

impl TextEngine {
    /// An engine with the built-in bitmap font registered as [`FontId(0)`](FontId), and a
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
            default: FontId::DEFAULT,
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

    /// Draws every style that names no font in this face.
    ///
    /// Without this, `FontId(0)` is the built-in 5x7 bitmap and there is no way
    /// to ask for anything else: every widget in this workspace carries
    /// [`TextStyle::built_in`] or [`TextStyle::default`], both of which name
    /// [`FontId::DEFAULT`], so registering a face with
    /// [`add_font`](TextEngine::add_font) alone registers something nothing
    /// refers to. That was [#130].
    ///
    /// One indirection, resolved when a style becomes glyphs — so no widget, no
    /// `TextStyle` and no form file changes, and an application that wants a
    /// real face says two lines instead of threading a style through everything
    /// it builds.
    ///
    /// An id that was never registered is ignored, because the alternative is a
    /// panel that draws nothing.
    ///
    /// ```
    /// # use denise_text::{FontId, TextEngine, TextStyle};
    /// let mut engine = TextEngine::new();
    /// // Nothing registered but the built-in, so the default is the built-in.
    /// assert_eq!(engine.default_font(), FontId::DEFAULT);
    ///
    /// // An id nobody registered changes nothing.
    /// engine.set_default_font(FontId(9));
    /// assert_eq!(engine.default_font(), FontId::DEFAULT);
    /// ```
    ///
    /// [#130]: https://github.com/bisand/denise/issues/130
    pub fn set_default_font(&mut self, font: FontId) {
        if (font.0 as usize) < self.sources.len() {
            self.default = font;
        }
    }

    /// Which face [`FontId::DEFAULT`] currently stands for.
    #[inline]
    pub const fn default_font(&self) -> FontId {
        self.default
    }

    /// The face a style is really drawn in.
    ///
    /// [`FontId::DEFAULT`] is a redirection rather than a face; everything else
    /// is itself. **Every lookup goes through here, and so does every glyph
    /// cache key** — a key built from the unresolved id would serve the old
    /// face's glyphs after the default changed.
    #[inline]
    const fn resolve(&self, font: FontId) -> FontId {
        if font.0 == FontId::DEFAULT.0 {
            self.default
        } else {
            font
        }
    }

    /// Number of registered fonts.
    #[inline]
    pub fn font_count(&self) -> usize {
        self.sources.len()
    }

    /// Name of a registered font.
    pub fn font_name(&self, font: FontId) -> Option<&str> {
        self.sources
            .get(self.resolve(font).0 as usize)
            .map(|s| s.name())
    }

    /// Whether a registered font has a glyph of its own for `ch`.
    ///
    /// The question an application asks before *choosing* what to draw: a
    /// keyboard that would like `⌫` on its Backspace key, a status line that
    /// would like `°`. Drawing a character the font lacks is not an error — it
    /// comes out as the missing-character box — so this is what turns a silent
    /// row of tofu into a legible fallback the author picked.
    ///
    /// `false` for an unregistered id, and for a font that can only render `ch`
    /// through shaping — see [`GlyphSource::glyph_id`].
    pub fn font_contains(&self, font: FontId, ch: char) -> bool {
        self.sources
            .get(self.resolve(font).0 as usize)
            .is_some_and(|s| s.contains(ch))
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
            .get(self.resolve(style.font).0 as usize)
            .map(|s| s.metrics(style.size_px))
            .unwrap_or_default()
    }

    /// The size this style will actually be drawn at.
    pub fn snap_size(&self, style: TextStyle) -> u16 {
        self.sources
            .get(self.resolve(style.font).0 as usize)
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
        let font = self.resolve(style.font);
        let Some(source) = self.sources.get_mut(font.0 as usize) else {
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
                font,
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
        let font = self.resolve(style.font);
        let source = self.sources.get_mut(font.0 as usize)?;
        let key = GlyphKey {
            font,
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

    /// The lines `text` becomes when broken to fit `max_width`.
    ///
    /// Greedy: words are added to a line until the next one would not fit. That
    /// is what every text editor does, it is one measuring pass, and the
    /// alternative — balancing lines by minimising raggedness — is a
    /// dynamic-programming problem this toolkit has no reason to solve.
    ///
    /// Explicit `\n` always breaks, so a caller who has already decided where
    /// the lines go keeps that decision.
    ///
    /// Slices borrow from `text`; nothing is copied. Words are separated by ASCII
    /// spaces, which is the boundary the built-in font can render and the one the
    /// languages this toolkit ships keyboard layouts for use.
    ///
    /// # A word wider than the line
    ///
    /// Goes on a line of its own and overflows, rather than being broken between
    /// characters. Breaking mid-word needs to know where a grapheme ends, and
    /// getting that wrong turns `æ` into two bytes of nothing — so an honest
    /// overflow the caller can see beats a corruption they cannot. A
    /// `max_width` of zero or less disables wrapping entirely for the same
    /// reason: there is no width that any word fits in.
    pub fn wrap<'a>(
        &mut self,
        style: TextStyle,
        text: &'a str,
        max_width: i32,
    ) -> alloc::vec::Vec<&'a str> {
        let mut lines = alloc::vec::Vec::new();
        for paragraph in text.split('\n') {
            if max_width <= 0 || paragraph.is_empty() {
                lines.push(paragraph);
                continue;
            }
            // Byte offsets into `paragraph`: `start` where the current line
            // begins, `end` where it currently ends. Both land on space
            // boundaries, which are ASCII and so always character boundaries.
            let mut start = 0;
            let mut end = 0;
            for (offset, word) in word_starts(paragraph) {
                let candidate = &paragraph[start..offset + word.len()];
                if end > start && self.measure_line(style, candidate) > max_width {
                    lines.push(&paragraph[start..end]);
                    start = offset;
                }
                end = offset + word.len();
            }
            lines.push(&paragraph[start..]);
        }
        lines
    }

    /// Height of `text` once wrapped to `max_width`.
    pub fn wrapped_height(&mut self, style: TextStyle, text: &str, max_width: i32) -> i32 {
        let lines = self.wrap(style, text, max_width).len() as i32;
        (lines * self.line_height(style)).max(0)
    }

    /// Draws one line with its baseline at `origin`.
    ///
    /// Returns the total advance.
    pub fn draw_line(
        &mut self,
        canvas: &mut Pen<'_>,
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
        canvas: &mut Pen<'_>,
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

#[cfg(test)]
mod wrap_tests {
    use super::*;
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;

    /// The built-in font is 8 px per character at 16 px, which makes every width
    /// in these tests a character count and the assertions readable.
    fn engine() -> (TextEngine, TextStyle) {
        (TextEngine::new(), TextStyle::built_in(16))
    }

    #[test]
    fn words_are_found_with_their_offsets_and_runs_of_spaces_collapse() {
        let words: Vec<_> = word_starts("ab cd  ef").collect();
        assert_eq!(words, vec![(0, "ab"), (3, "cd"), (7, "ef")]);
        assert_eq!(word_starts("").count(), 0);
        assert_eq!(word_starts("   ").count(), 0);
        let single: Vec<_> = word_starts("  x").collect();
        assert_eq!(single, vec![(2, "x")]);
    }

    /// The ordinary case: greedy fill, breaking where the next word would not
    /// fit.
    #[test]
    fn a_line_breaks_where_the_next_word_would_not_fit() {
        let (mut engine, style) = engine();
        let lines = engine.wrap(style, "en to tre fire fem", 80);
        let widths: Vec<i32> = lines
            .iter()
            .map(|l| engine.measure_line(style, l))
            .collect();
        for (line, width) in lines.iter().zip(&widths) {
            assert!(
                *width <= 80 || !line.contains(' '),
                "{line:?} is {width} wide and could have been broken"
            );
        }
        assert!(lines.len() > 1, "nothing wrapped at all");
        assert_eq!(lines.concat().replace(' ', ""), "entotrefirefem");
    }

    /// Explicit breaks are a decision the caller already made.
    #[test]
    fn an_explicit_newline_always_breaks() {
        let (mut engine, style) = engine();
        assert_eq!(engine.wrap(style, "a\nb\nc", 10_000), vec!["a", "b", "c"]);
    }

    /// A word wider than the line overflows on its own line rather than being
    /// cut between bytes — `æ` is two of them, and half of it is nothing.
    #[test]
    fn a_word_wider_than_the_line_gets_its_own_line_and_overflows() {
        let (mut engine, style) = engine();
        let input = "kort kjempelangtordherinne kort";
        let lines = engine.wrap(style, input, 40);
        assert!(
            lines.contains(&"kjempelangtordherinne"),
            "the long word was broken or lost: {lines:?}"
        );
        // Against the input rather than a number I worked out by hand, which is
        // how this assertion was wrong the first time.
        assert_eq!(lines.concat().replace(' ', ""), input.replace(' ', ""));
    }

    /// No width is not a width every word fails to fit; it is no wrapping.
    #[test]
    fn a_width_of_zero_or_less_does_not_wrap() {
        let (mut engine, style) = engine();
        assert_eq!(engine.wrap(style, "en to tre", 0), vec!["en to tre"]);
        assert_eq!(engine.wrap(style, "en to tre", -5), vec!["en to tre"]);
    }

    /// Empty input is one empty line, not no lines — a blank paragraph still
    /// occupies a line's height.
    #[test]
    fn empty_text_is_one_empty_line() {
        let (mut engine, style) = engine();
        assert_eq!(engine.wrap(style, "", 100), vec![""]);
        assert_eq!(engine.wrap(style, "\n", 100), vec!["", ""]);
    }

    /// Nothing is dropped and nothing is duplicated, at any width. The property
    /// that matters: wrapping rearranges, it does not edit.
    #[test]
    fn wrapping_never_loses_or_duplicates_a_character() {
        let (mut engine, style) = engine();
        let text = "Kjærlighet på Øy er en lang setning med æøå i seg";
        let stripped: String = text.chars().filter(|c| *c != ' ').collect();
        for width in [1, 8, 17, 40, 101, 500, 5000] {
            let joined = engine.wrap(style, text, width).concat();
            let got: String = joined.chars().filter(|c| *c != ' ').collect();
            assert_eq!(got, stripped, "width {width} changed the text");
        }
    }

    /// The height follows the line count, which is what a widget sizes itself
    /// from.
    #[test]
    fn the_wrapped_height_is_the_line_count_times_the_line_height() {
        let (mut engine, style) = engine();
        let one = engine.wrapped_height(style, "kort", 10_000);
        assert_eq!(one, engine.line_height(style));
        let many = engine.wrapped_height(style, "en to tre fire fem seks sju", 40);
        assert!(many > one, "wrapped text should be taller");
        assert_eq!(many % engine.line_height(style), 0);
    }
}
