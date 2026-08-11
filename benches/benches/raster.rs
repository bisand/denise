//! Cost of the individual drawing primitives.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use denise::{Color, Point, Rect, Size};
use denise_benches::{PANEL, SMALL_PANEL, Target};
use std::hint::black_box;

fn clear(c: &mut Criterion) {
    let mut group = c.benchmark_group("clear");
    for size in [SMALL_PANEL, PANEL] {
        let mut target = Target::new(size);
        group.throughput(Throughput::Elements(size.area()));
        group.bench_function(BenchmarkId::from_parameter(label(size)), |b| {
            b.iter(|| {
                target
                    .canvas()
                    .clear(black_box(Color::from_rgb888(0x1E1E2E)))
            });
        });
    }
    group.finish();
}

fn clear_with_pitch_padding(c: &mut Criterion) {
    // A DRM framebuffer is pitch-aligned, so the fast path has to work per row
    // rather than over one contiguous slice. This is the honest number.
    let mut group = c.benchmark_group("clear_padded");
    let size = PANEL;
    let mut target = Target::with_stride(size, size.width + 64);
    group.throughput(Throughput::Elements(size.area()));
    group.bench_function(label(size), |b| {
        b.iter(|| {
            target
                .canvas()
                .clear(black_box(Color::from_rgb888(0x1E1E2E)))
        });
    });
    group.finish();
}

fn fill_rect(c: &mut Criterion) {
    let size = PANEL;
    let rect = Rect::new(160, 90, 1600, 900);
    let mut target = Target::new(size);

    let mut group = c.benchmark_group("fill_rect");
    group.throughput(Throughput::Elements(rect.area()));
    group.bench_function("opaque", |b| {
        b.iter(|| {
            target
                .canvas()
                .fill_rect(black_box(rect), Color::from_rgb888(0x313244))
        });
    });
    group.bench_function("alpha", |b| {
        b.iter(|| {
            target
                .canvas()
                .fill_rect(black_box(rect), Color::rgba(0, 0, 0, 128))
        });
    });
    group.finish();
}

fn rounded_rect(c: &mut Criterion) {
    let rect = Rect::new(160, 90, 1600, 900);
    let mut target = Target::new(PANEL);

    let mut group = c.benchmark_group("rounded_rect");
    group.throughput(Throughput::Elements(rect.area()));
    for radius in [8, 32] {
        group.bench_with_input(BenchmarkId::new("fill", radius), &radius, |b, &radius| {
            b.iter(|| {
                target.canvas().fill_rounded_rect(
                    black_box(rect),
                    radius,
                    Color::from_rgb888(0x313244),
                )
            });
        });
        group.bench_with_input(BenchmarkId::new("stroke", radius), &radius, |b, &radius| {
            b.iter(|| {
                target.canvas().stroke_rounded_rect(
                    black_box(rect),
                    radius,
                    2,
                    Color::from_rgb888(0x89B4FA),
                )
            });
        });
    }
    group.finish();
}

fn line(c: &mut Criterion) {
    let mut target = Target::new(PANEL);
    let mut group = c.benchmark_group("line");

    group.throughput(Throughput::Elements(1720));
    group.bench_function("axis_aligned", |b| {
        b.iter(|| {
            target.canvas().draw_line(
                black_box(Point::new(100, 540)),
                black_box(Point::new(1820, 540)),
                Color::WHITE,
            )
        });
    });
    group.bench_function("antialiased", |b| {
        b.iter(|| {
            target.canvas().draw_line(
                black_box(Point::new(100, 100)),
                black_box(Point::new(1820, 980)),
                Color::WHITE,
            )
        });
    });
    group.finish();
}

fn label(size: Size) -> String {
    format!("{}x{}", size.width, size.height)
}

criterion_group!(
    benches,
    clear,
    clear_with_pitch_padding,
    fill_rect,
    rounded_rect,
    line
);
criterion_main!(benches);
