//! The glyph cache and the engine, driven the way a widget drives them.

use denise::{BufferAge, Color, Frame, PixelFormat, Point, Rect, Size};
use denise_render::Canvas;
use denise_text::atlas::GlyphKey;
use denise_text::{BitmapSource, FontId, GlyphAtlas, GlyphSource, TextEngine, TextStyle};

const SURFACE: Size = Size::new(240, 80);

/// A canvas over a buffer the test can inspect afterwards.
struct Sheet {
    pixels: Vec<u32>,
}

impl Sheet {
    fn new() -> Self {
        Self {
            pixels: vec![0; (SURFACE.width * SURFACE.height) as usize],
        }
    }

    fn draw(&mut self, f: impl FnOnce(&mut Canvas<'_>)) {
        let mut frame = Frame::new(
            &mut self.pixels,
            SURFACE,
            SURFACE.width,
            PixelFormat::Xrgb8888,
            BufferAge::Undefined,
        )
        .expect("frame");
        let mut canvas = Canvas::new(&mut frame);
        f(&mut canvas);
    }

    /// Bounding box of everything non-black, or `None` if nothing was drawn.
    fn ink_bounds(&self) -> Option<Rect> {
        let mut bounds: Option<Rect> = None;
        for y in 0..SURFACE.height as i32 {
            for x in 0..SURFACE.width as i32 {
                if self.pixels[(y * SURFACE.width as i32 + x) as usize] == 0 {
                    continue;
                }
                let pixel = Rect::new(x, y, 1, 1);
                bounds = Some(match bounds {
                    Some(b) => b.union(&pixel),
                    None => pixel,
                });
            }
        }
        bounds
    }

    fn is_blank(&self) -> bool {
        self.pixels.iter().all(|&p| p == 0)
    }
}

fn engine() -> TextEngine {
    TextEngine::new()
}

#[test]
fn the_built_in_font_is_always_font_zero() {
    let engine = engine();
    assert_eq!(engine.font_count(), 1);
    assert_eq!(engine.font_name(FontId(0)), Some("built-in 5x7"));
    assert_eq!(engine.font_name(FontId(7)), None);
}

#[test]
fn a_second_lookup_of_the_same_glyph_hits_the_cache() {
    let mut engine = engine();
    let style = TextStyle::built_in(16);
    let mut sheet = Sheet::new();

    sheet.draw(|canvas| {
        engine.draw(canvas, style, Point::new(4, 4), "aaa", Color::WHITE);
    });
    let after_first = engine.stats();
    assert_eq!(
        after_first.misses, 1,
        "one distinct glyph, one rasterisation"
    );
    assert_eq!(after_first.hits, 2);

    sheet.draw(|canvas| {
        engine.draw(canvas, style, Point::new(4, 40), "aaa", Color::WHITE);
    });
    let after_second = engine.stats();
    assert_eq!(
        after_second.misses, 1,
        "drawing the same text again must not rasterise anything"
    );
    assert_eq!(after_second.hits, 5);
    assert_eq!(after_second.hit_rate(), Some(83));
}

#[test]
fn the_same_glyph_at_two_sizes_is_two_entries() {
    let mut engine = engine();
    let mut sheet = Sheet::new();
    sheet.draw(|canvas| {
        engine.draw(
            canvas,
            TextStyle::built_in(8),
            Point::new(4, 4),
            "A",
            Color::WHITE,
        );
        engine.draw(
            canvas,
            TextStyle::built_in(24),
            Point::new(4, 20),
            "A",
            Color::WHITE,
        );
    });
    assert_eq!(engine.stats().misses, 2);
    assert_eq!(engine.atlas().len(), 2);
}

#[test]
fn measurement_agrees_with_where_the_ink_lands() {
    let mut engine = engine();
    let style = TextStyle::built_in(16);
    let origin = Point::new(20, 20);
    let extent = engine.measure(style, "Hg");

    let mut sheet = Sheet::new();
    sheet.draw(|canvas| {
        engine.draw(canvas, style, origin, "Hg", Color::WHITE);
    });

    let ink = sheet.ink_bounds().expect("something was drawn");
    let box_ = Rect::new(
        origin.x,
        origin.y,
        extent.width as i32,
        extent.height as i32,
    );
    assert!(
        box_.contains_rect(&ink),
        "ink {ink:?} escaped the measured box {box_:?}"
    );
    // `Hg` has a descender, so it should use most of the measured height.
    assert!(
        ink.height * 2 > box_.height,
        "ink {ink:?} is suspiciously short inside {box_:?}"
    );
}

#[test]
fn a_labels_height_does_not_change_with_its_letters() {
    // A readout that gains a descender must not reflow the form around it.
    let mut engine = engine();
    let style = TextStyle::built_in(16);
    let flat = engine.measure(style, "one");
    let deep = engine.measure(style, "ogg");
    assert_eq!(flat.height, deep.height);
    assert_eq!(flat.width, deep.width, "the built-in font is monospace");
}

#[test]
fn a_newline_starts_a_second_line() {
    let mut engine = engine();
    let style = TextStyle::built_in(8);
    let one = engine.measure(style, "AA");
    let two = engine.measure(style, "A\nA");
    assert_eq!(two.height, one.height * 2);
    assert!(two.width < one.width, "two lines of one glyph are narrower");
}

#[test]
fn nordic_letters_survive_the_whole_path() {
    let mut engine = engine();
    let style = TextStyle::built_in(16);
    let mut sheet = Sheet::new();
    sheet.draw(|canvas| {
        engine.draw(canvas, style, Point::new(4, 4), "æøå", Color::WHITE);
    });
    assert!(!sheet.is_blank(), "æøå must reach the screen");
    assert_eq!(engine.stats().misses, 3);
    assert_eq!(engine.stats().failures, 0);
}

#[test]
fn a_space_is_cached_without_taking_atlas_space() {
    let mut atlas = GlyphAtlas::new(Size::new(64, 64));
    let mut source = BitmapSource::new();
    let key = GlyphKey {
        font: FontId(0),
        size_px: 16,
        ch: ' ',
    };
    let placed = atlas.get_or_insert(key, &mut source).expect("space");
    assert!(placed.rect.is_empty());
    assert!(placed.metrics.advance > 0, "a space still moves the pen");
    assert!(atlas.mask(&placed).is_none(), "there is nothing to blit");
}

#[test]
fn packed_glyphs_never_overlap() {
    let mut atlas = GlyphAtlas::new(Size::new(128, 128));
    let mut source = BitmapSource::new();
    let mut placed = Vec::new();
    for ch in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789".chars() {
        let key = GlyphKey {
            font: FontId(0),
            size_px: 16,
            ch,
        };
        if let Some(p) = atlas.get_or_insert(key, &mut source) {
            placed.push((ch, p.rect));
        }
    }
    assert!(placed.len() > 30, "most of those should have fitted");
    for (i, (a, ra)) in placed.iter().enumerate() {
        assert!(
            Rect::from_size(atlas.size()).contains_rect(ra),
            "{a:?} at {ra:?} is outside the atlas"
        );
        for (b, rb) in &placed[..i] {
            assert!(
                !ra.intersects(rb),
                "{a:?} at {ra:?} overlaps {b:?} at {rb:?}"
            );
        }
    }
}

#[test]
fn a_full_atlas_resets_instead_of_failing() {
    // Deliberately far too small for the alphabet at this size.
    let mut atlas = GlyphAtlas::new(Size::new(32, 32));
    let mut source = BitmapSource::new();
    let mut drawn = 0;
    for ch in "ABCDEFGHIJKLMNOPQRSTUVWXYZ".chars() {
        let key = GlyphKey {
            font: FontId(0),
            size_px: 24,
            ch,
        };
        if atlas.get_or_insert(key, &mut source).is_some() {
            drawn += 1;
        }
    }
    assert_eq!(drawn, 26, "every glyph must still be produced");
    assert!(
        atlas.stats().resets > 0,
        "a cache this small has to have been cleared"
    );
    assert_eq!(
        atlas.stats().failures,
        0,
        "resetting is not failing; the glyphs all fit individually"
    );
}

#[test]
fn a_glyph_larger_than_the_whole_atlas_fails_without_looping() {
    let mut atlas = GlyphAtlas::new(Size::new(4, 4));
    let mut source = BitmapSource::new();
    let key = GlyphKey {
        font: FontId(0),
        size_px: 48,
        ch: 'M',
    };
    assert_eq!(atlas.get_or_insert(key, &mut source), None);
    assert_eq!(atlas.stats().failures, 1);
}

#[test]
fn the_mask_of_a_placed_glyph_is_the_glyph() {
    let mut atlas = GlyphAtlas::new(Size::new(64, 64));
    let mut source = BitmapSource::new();

    // Two glyphs, so the second one is not at the atlas origin and a mask that
    // ignored the offset would return the first one's pixels.
    for ch in ['I', 'M'] {
        let key = GlyphKey {
            font: FontId(0),
            size_px: 8,
            ch,
        };
        atlas.get_or_insert(key, &mut source).expect("glyph");
    }

    let key = GlyphKey {
        font: FontId(0),
        size_px: 8,
        ch: 'M',
    };
    let placed = atlas.get_or_insert(key, &mut source).expect("M");
    let mask = atlas.mask(&placed).expect("M has ink");
    assert_eq!(mask.width(), placed.metrics.size.width as i32);
    assert_eq!(mask.height(), placed.metrics.size.height as i32);

    let direct = source.rasterise('M', 8).expect("M");
    let mut sheet_a = Sheet::new();
    sheet_a.draw(|canvas| canvas.blit_mask(Point::new(10, 10), &mask, Color::WHITE));
    let mut sheet_b = Sheet::new();
    sheet_b.draw(|canvas| {
        let direct_mask = denise_render::Mask::new(
            direct.coverage,
            direct.metrics.size.width as i32,
            direct.metrics.size.height as i32,
            direct.stride,
        )
        .expect("mask");
        canvas.blit_mask(Point::new(10, 10), &direct_mask, Color::WHITE);
    });
    assert_eq!(
        sheet_a.pixels, sheet_b.pixels,
        "the cached glyph must be pixel-identical to a fresh rasterisation"
    );
}

#[test]
fn the_cache_is_bounded_by_what_it_was_asked_for() {
    let atlas = GlyphAtlas::new(Size::new(256, 256));
    assert_eq!(atlas.capacity_bytes(), 65_536);
    assert_eq!(GlyphAtlas::with_default_size().capacity_bytes(), 65_536);
}

#[test]
fn drawing_off_the_surface_is_harmless() {
    let mut engine = engine();
    let style = TextStyle::built_in(16);
    let mut sheet = Sheet::new();
    sheet.draw(|canvas| {
        engine.draw(
            canvas,
            style,
            Point::new(-500, -500),
            "clipped",
            Color::WHITE,
        );
        engine.draw(
            canvas,
            style,
            Point::new(5000, 5000),
            "clipped",
            Color::WHITE,
        );
    });
    assert!(sheet.is_blank());
}

#[test]
fn an_unknown_font_id_draws_nothing_and_measures_zero() {
    let mut engine = engine();
    let style = TextStyle {
        font: FontId(9),
        size_px: 16,
    };
    assert_eq!(engine.measure_line(style, "hello"), 0);
    let mut sheet = Sheet::new();
    sheet.draw(|canvas| {
        engine.draw(canvas, style, Point::new(4, 4), "hello", Color::WHITE);
    });
    assert!(sheet.is_blank());
}

#[test]
fn layout_positions_match_the_advances() {
    let mut engine = engine();
    let style = TextStyle::built_in(16);
    let mut pens = Vec::new();
    let total = engine.layout_line(style, "abc", |glyph| pens.push((glyph.ch, glyph.pen_x)));
    assert_eq!(pens.len(), 3);
    assert_eq!(pens[0].1, 0);
    assert!(pens[1].1 > pens[0].1 && pens[2].1 > pens[1].1);
    assert!(total > pens[2].1, "the last glyph still advances the pen");
}
