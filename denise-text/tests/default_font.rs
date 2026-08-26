//! The face a style that names none is drawn in.
//!
//! Every `TextStyle` this workspace builds carries [`FontId::DEFAULT`] — that is
//! what `TextStyle::built_in` and `TextStyle::default` both name — so before
//! there was a way to point that somewhere else, registering a font registered
//! something nothing referred to. A form could only ever be drawn in the
//! built-in 5x7 bitmap, whatever the machine had installed. That was [#130].
//!
//! These use a stub source rather than a real `.ttf`, deliberately: what is under
//! test is the indirection, and a test that needed a font file would be a test
//! that only ran where somebody had installed one.
//!
//! [#130]: https://github.com/bisand/denise/issues/130

use denise::Size;
use denise_text::{
    FontId, FontMetrics, GlyphId, GlyphMetrics, GlyphSource, Rasterised, TextEngine, TextStyle,
};

/// A face where every glyph is a solid square of a chosen width.
///
/// Nothing like a real font, and it does not need to be: it measures and
/// rasterises differently from the built-in one, which is the entire question.
struct Blocks {
    name: String,
    advance: i32,
    ink: Vec<u8>,
}

impl Blocks {
    fn new(name: &str, advance: i32) -> Self {
        Self {
            name: name.to_string(),
            advance,
            ink: vec![255; 64 * 64],
        }
    }
}

impl GlyphSource for Blocks {
    fn name(&self) -> &str {
        &self.name
    }

    fn metrics(&self, size_px: u16) -> FontMetrics {
        FontMetrics {
            ascent: i32::from(size_px),
            descent: 0,
            line_gap: 0,
        }
    }

    fn glyph_id(&self, ch: char) -> Option<GlyphId> {
        Some(GlyphId(ch as u32))
    }

    fn contains(&self, _ch: char) -> bool {
        true
    }

    fn glyph_metrics(&mut self, _glyph: GlyphId, size_px: u16) -> Option<GlyphMetrics> {
        let side = i32::from(size_px).clamp(1, 64);
        Some(GlyphMetrics {
            advance: self.advance,
            bearing_x: 0,
            bearing_y: side,
            size: Size::new(side as u32, side as u32),
        })
    }

    fn rasterise(&mut self, glyph: GlyphId, size_px: u16) -> Option<Rasterised<'_>> {
        let metrics = self.glyph_metrics(glyph, size_px)?;
        Some(Rasterised {
            metrics,
            coverage: &self.ink,
            stride: 64,
        })
    }
}

#[test]
fn the_built_in_face_is_the_default_until_somebody_says_otherwise() {
    // What a board with no fonts installed gets, and why `FontId(0)` is always
    // the bitmap face.
    let engine = TextEngine::new();
    assert_eq!(engine.default_font(), FontId::DEFAULT);
    assert_eq!(engine.font_count(), 1);
}

#[test]
fn a_registered_face_changes_nothing_until_it_is_made_the_default() {
    // The bug, stated: `add_font` alone registers a face nothing refers to,
    // because every style in the workspace names `FontId::DEFAULT`.
    let mut engine = TextEngine::new();
    let style = TextStyle::built_in(16);
    let before = engine.measure_line(style, "Hello");

    let blocks = engine.add_font(Box::new(Blocks::new("Blocks", 40)));
    assert_eq!(
        engine.measure_line(style, "Hello"),
        before,
        "registering a face changed what an unnamed style measures",
    );

    // And the fix: one line, and every style that names no font follows.
    engine.set_default_font(blocks);
    assert_eq!(
        engine.measure_line(style, "Hello"),
        40 * 5,
        "the default face did not take effect",
    );
}

#[test]
fn a_style_that_names_a_face_is_unaffected_by_the_default() {
    // The redirection is `FontId::DEFAULT` and nothing else: a style that asked
    // for a particular face gets it, whatever the default is.
    let mut engine = TextEngine::new();
    let wide = engine.add_font(Box::new(Blocks::new("Wide", 40)));
    let narrow = engine.add_font(Box::new(Blocks::new("Narrow", 10)));

    engine.set_default_font(wide);
    let named = TextStyle {
        font: narrow,
        size_px: 16,
    };
    assert_eq!(engine.measure_line(named, "abc"), 30);
    assert_eq!(engine.measure_line(TextStyle::built_in(16), "abc"), 120);
}

#[test]
fn the_glyph_cache_does_not_serve_the_old_face_after_the_default_moves() {
    // The trap in this change. A cache key built from the *unresolved* id would
    // be the same key before and after, and every glyph already drawn would
    // keep the old face's shape.
    let mut engine = TextEngine::new();
    let wide = engine.add_font(Box::new(Blocks::new("Wide", 40)));
    let narrow = engine.add_font(Box::new(Blocks::new("Narrow", 10)));
    let style = TextStyle::built_in(16);

    engine.set_default_font(wide);
    assert_eq!(engine.measure_line(style, "abcd"), 160);

    // Same style, same text, same size — and it must now measure the other face.
    engine.set_default_font(narrow);
    assert_eq!(
        engine.measure_line(style, "abcd"),
        40,
        "the cache served the face that used to be the default",
    );
}

#[test]
fn an_id_nobody_registered_is_ignored_rather_than_drawing_nothing() {
    let mut engine = TextEngine::new();
    let style = TextStyle::built_in(16);
    let before = engine.measure_line(style, "Hello");

    engine.set_default_font(FontId(200));
    assert_eq!(engine.default_font(), FontId::DEFAULT);
    assert_eq!(engine.measure_line(style, "Hello"), before);
}

#[test]
fn the_name_and_the_metrics_follow_the_default_too() {
    // Not only the glyphs: everything asked about an unnamed style resolves the
    // same way, or a caller would measure one face and draw another.
    let mut engine = TextEngine::new();
    let blocks = engine.add_font(Box::new(Blocks::new("Blocks", 20)));
    engine.set_default_font(blocks);

    assert_eq!(engine.font_name(FontId::DEFAULT), Some("Blocks"));
    assert_eq!(engine.metrics(TextStyle::built_in(16)).ascent, 16);
    assert!(engine.font_contains(FontId::DEFAULT, 'q'));
}
