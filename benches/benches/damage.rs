//! The benchmark the project exists to win.
//!
//! `scene/full` repaints a 1080p UI. `scene/damaged` repaints the same UI through
//! a typical damage set. If the ratio between them is not enormous, dirty-rect
//! tracking is not earning its complexity and something upstream is broken.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use denise::{Rect, Size};
use denise_benches::{PANEL, Target, paint_scene, scene, typical_damage};
use std::hint::black_box;

fn scene_repaint(c: &mut Criterion) {
    let size = PANEL;
    let widgets = scene(size);
    let damage = typical_damage(size);
    let damaged_pixels: u64 = damage.iter().map(Rect::area).sum();

    let mut target = Target::new(size);
    let mut group = c.benchmark_group("scene");

    group.throughput(Throughput::Elements(size.area()));
    group.bench_function("full", |b| {
        b.iter(|| paint_scene(&mut target.canvas(), black_box(&widgets)));
    });

    group.throughput(Throughput::Elements(damaged_pixels));
    group.bench_function("damaged", |b| {
        b.iter(|| {
            let mut canvas = target.canvas();
            for region in black_box(&damage) {
                let mut clipped = canvas.with_clip(*region);
                paint_scene(&mut clipped, &widgets);
            }
        });
    });

    group.finish();
}

fn blit(c: &mut Criterion) {
    // What a double-buffered backend pays to publish a frame: copy the shadow
    // buffer to the scanout buffer, either wholesale or only where it changed.
    let size = PANEL;
    let source = Target::new(size);
    let view = source.view();
    let damage = typical_damage(size);
    let damaged_pixels: u64 = damage.iter().map(Rect::area).sum();
    let whole = [Rect::from_size(size)];

    let mut target = Target::new(size);
    let mut group = c.benchmark_group("blit");

    group.throughput(Throughput::Elements(size.area()));
    group.bench_function("full", |b| {
        b.iter(|| target.canvas().copy_from(&view, black_box(&whole)));
    });

    group.throughput(Throughput::Elements(damaged_pixels));
    group.bench_function("damaged", |b| {
        b.iter(|| target.canvas().copy_from(&view, black_box(&damage)));
    });

    group.finish();
}

fn small_panel(c: &mut Criterion) {
    // The size these actually ship at. If a full repaint here does not fit
    // comfortably inside a 16.6 ms frame, nothing else in the project matters.
    let size = Size::new(800, 480);
    let widgets = scene(size);
    let mut target = Target::new(size);

    let mut group = c.benchmark_group("pi_panel");
    group.throughput(Throughput::Elements(size.area()));
    group.bench_function("full", |b| {
        b.iter(|| paint_scene(&mut target.canvas(), black_box(&widgets)));
    });
    group.finish();
}

criterion_group!(benches, scene_repaint, blit, small_panel);
criterion_main!(benches);
