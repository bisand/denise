//! Glyph cache hit and miss, and what a line of text costs.
//!
//! The bootstrap asked for hit and miss separately, and the ratio between them is
//! the number that decides whether the cache earns its 64 KB. The third group is
//! the one that matters in practice: a panel redraws the same labels every frame,
//! so almost every lookup is a hit and the miss cost is paid once at boot.

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use denise::{BufferAge, Color, Frame, PixelFormat, Point, Size};
use denise_render::Canvas;
use denise_text::atlas::GlyphKey;
use denise_text::{BitmapSource, FontId, GlyphAtlas, TextEngine, TextStyle};

const SURFACE: Size = Size::new(1920, 1080);
const LABEL: &str = "Kjærlighet på Øy";
const PARAGRAPH: &str = "Vår sære Zulu fra badeøya spilte jo whist og quickstep i min taxi.";

fn cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("glyph_cache");

    group.bench_function("hit", |b| {
        let mut atlas = GlyphAtlas::with_default_size();
        let mut source = BitmapSource::new();
        let key = GlyphKey::from_char(FontId(0), 16, 'a');
        atlas.get_or_insert(key, &mut source).expect("warm");
        b.iter(|| black_box(atlas.get_or_insert(black_box(key), &mut source)))
    });

    group.bench_function("miss", |b| {
        let mut atlas = GlyphAtlas::with_default_size();
        let mut source = BitmapSource::new();
        let key = GlyphKey::from_char(FontId(0), 16, 'a');
        b.iter(|| {
            // Clearing is what makes the next lookup a miss. It also clears the
            // shelves, so this measures rasterise-plus-pack, which is what a cold
            // panel actually pays.
            atlas.clear();
            black_box(atlas.get_or_insert(black_box(key), &mut source))
        })
    });

    group.bench_function("miss, 24 px", |b| {
        let mut atlas = GlyphAtlas::with_default_size();
        let mut source = BitmapSource::new();
        let key = GlyphKey::from_char(FontId(0), 24, 'a');
        b.iter(|| {
            atlas.clear();
            black_box(atlas.get_or_insert(black_box(key), &mut source))
        })
    });

    group.finish();
}

fn measurement(c: &mut Criterion) {
    let mut engine = TextEngine::new();
    let style = TextStyle::built_in(16);
    // Warm, because a panel measures the same strings on every layout pass.
    engine.measure(style, PARAGRAPH);

    let mut group = c.benchmark_group("measure");
    group.bench_function("label, 16 chars", |b| {
        b.iter(|| black_box(engine.measure(black_box(style), black_box(LABEL))))
    });
    group.bench_function("paragraph, 66 chars", |b| {
        b.iter(|| black_box(engine.measure(black_box(style), black_box(PARAGRAPH))))
    });
    group.finish();
}

fn drawing(c: &mut Criterion) {
    let mut pixels = vec![0u32; (SURFACE.width * SURFACE.height) as usize];
    let mut engine = TextEngine::new();

    let mut group = c.benchmark_group("draw_line");
    for size in [16u16, 24, 48] {
        let style = TextStyle::built_in(size);
        group.bench_function(format!("{size} px, warm cache"), |b| {
            // Warm inside the closure so the first iteration is not the odd one.
            let mut frame = Frame::new(
                &mut pixels,
                SURFACE,
                SURFACE.width,
                PixelFormat::Xrgb8888,
                BufferAge::Frames(1),
            )
            .expect("frame");
            let mut raster = Canvas::new(&mut frame);
            let mut canvas = raster.pen();
            engine.draw(&mut canvas, style, Point::new(40, 40), LABEL, Color::WHITE);
            b.iter(|| {
                black_box(engine.draw(
                    &mut canvas,
                    black_box(style),
                    Point::new(40, 40),
                    black_box(LABEL),
                    Color::WHITE,
                ))
            })
        });
    }
    group.finish();
}

criterion_group!(benches, cache, measurement, drawing);
criterion_main!(benches);
