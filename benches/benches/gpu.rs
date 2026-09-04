//! What the GPU path costs against the software one, and how each scales.
//!
//! The interesting number is not a single frame time — it is the *slope*. The
//! rasteriser's cost is per pixel: four times the area is four times the work,
//! whatever is on the screen. The GPU's is per primitive: the same tree at four
//! times the area is the same vertices, and the fragments are somebody else's
//! problem. So the two are measured at 1080p and at 4K, and the pair of numbers
//! says which side of the crossover a given window is on.
//!
//! Both paths repaint everything here, which is the fair comparison and not the
//! usual one: in a real application the rasteriser repaints only the damage,
//! and the GPU repaints the window every frame because a swapchain remembers
//! nothing. The `damaged` case measures that difference, and it is the one that
//! decides whether a GPU is worth it for an idle panel — the answer being no,
//! which is why the kiosk path never grew one.
//!
//! Two GPU numbers are taken because they answer different questions.
//! `encode` is what the CPU pays to record and submit a frame, which is what
//! decides whether an application can keep up. `to completion` waits for the
//! device, which is the frame's latency. On a desktop the first is the one that
//! competes with the rasteriser for a core.
//!
//! Runs only where wgpu finds an adapter. Everywhere else — a CI runner without
//! a software Vulkan — the GPU groups are skipped, loudly.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};

use denise::{BufferAge, Frame, InputEvent, Pen, PixelFormat, Point, Rect, Size, theme};
use denise_ui::widgets::{Button, Label, Panel};
use denise_ui::{NodeId, Ui};
use denise_wgpu::Gpu;

/// A busy HMI: 20 panels of 24 controls, a little over five hundred nodes. The
/// same caricature the `ui` bench uses, so the software numbers here and there
/// are the same measurement.
const PANELS_X: i32 = 5;
const PANELS_Y: i32 = 4;
const CONTROLS: i32 = 12;

/// The two sizes the question turns on: a panel, and a Retina designer window.
const HD: Size = Size::new(1920, 1080);
const UHD: Size = Size::new(3840, 2160);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Msg {
    Pressed(u32),
}

fn busy_panel(size: Size) -> (Ui<Msg>, Vec<NodeId>) {
    let mut ui: Ui<Msg> = Ui::new(size, theme::DARK);
    let root = ui.root();
    let mut buttons = Vec::new();
    let panel_w = size.width as i32 / PANELS_X;
    let panel_h = size.height as i32 / PANELS_Y;

    let mut index = 0u32;
    for py in 0..PANELS_Y {
        for px in 0..PANELS_X {
            let panel = ui
                .add(
                    root,
                    Panel::default(),
                    Rect::new(px * panel_w + 4, py * panel_h + 4, panel_w - 8, panel_h - 8),
                )
                .expect("panel");
            for i in 0..CONTROLS {
                let row = i / 2;
                let column = i % 2;
                let cell = Rect::new(
                    12 + column * (panel_w / 2 - 8),
                    12 + row * 36,
                    panel_w / 2 - 24,
                    30,
                );
                if column == 0 {
                    ui.add(panel, Label::new("Setpoint"), cell).expect("label");
                } else {
                    buttons.push(
                        ui.add(panel, Button::new("Apply", Msg::Pressed(index)), cell)
                            .expect("button"),
                    );
                }
                index += 1;
            }
        }
    }
    (ui, buttons)
}

fn centre(ui: &Ui<Msg>, id: NodeId) -> Point {
    let bounds = ui.bounds(id).expect("bounds");
    Point::new(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2)
}

/// The rasteriser, repainting everything, at one size.
fn software(c: &mut Criterion, size: Size, label: &str) {
    let (mut ui, buttons) = busy_panel(size);
    let mut pixels = vec![0u32; (size.width * size.height) as usize];

    let mut group = c.benchmark_group(format!("frame_{label}"));
    group.bench_function("software, full repaint", |b| {
        b.iter(|| {
            ui.invalidate_all();
            let mut frame = Frame::new(
                &mut pixels,
                size,
                size.width,
                PixelFormat::Xrgb8888,
                BufferAge::Undefined,
            )
            .expect("frame");
            ui.paint(&mut frame);
            drop(frame);
            ui.presented();
        })
    });

    // What the rasteriser actually does in an application: repaint the two
    // rectangles a moved pointer dirtied, and nothing else. The GPU has no
    // counterpart, which is the whole point of the comparison.
    let at = centre(&ui, buttons[7]);
    let elsewhere = Point::new(at.x, at.y + 400);
    group.bench_function("software, damaged", |b| {
        b.iter(|| {
            ui.handle(&[InputEvent::PointerMoved { position: at }]);
            ui.handle(&[InputEvent::PointerMoved {
                position: elsewhere,
            }]);
            let mut frame = Frame::new(
                &mut pixels,
                size,
                size.width,
                PixelFormat::Xrgb8888,
                BufferAge::Frames(1),
            )
            .expect("frame");
            ui.paint(&mut frame);
            drop(frame);
            ui.presented();
        })
    });
    group.finish();
}

/// The GPU, repainting everything, at one size. Skipped with no adapter.
fn gpu(c: &mut Criterion, size: Size, label: &str) {
    let gpu = match Gpu::headless() {
        Ok(gpu) => gpu,
        Err(err) => {
            eprintln!("skipping the GPU groups: {err}");
            return;
        }
    };
    let (mut ui, buttons) = busy_panel(size);

    // One offscreen target, made once: a swapchain's texture in a window, and
    // not part of what is being timed either way.
    let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("bench target"),
        size: wgpu::Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: gpu.format(),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    // The glyph atlas fills on the first frame; a bench of the second frame
    // onwards is what a running application pays.
    {
        let mut painter = gpu.painter(size);
        ui.paint_with(&mut Pen::new(&mut painter), BufferAge::Undefined);
        painter.finish(&view);
        gpu.device()
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("warm-up");
        ui.presented();
    }
    let uploads = gpu.page_uploads();

    let mut group = c.benchmark_group(format!("frame_{label}"));
    group.bench_function("gpu, full repaint (encode + submit)", |b| {
        b.iter(|| {
            ui.invalidate_all();
            let mut painter = gpu.painter(size);
            ui.paint_with(&mut Pen::new(&mut painter), BufferAge::Undefined);
            painter.finish(black_box(&view));
            ui.presented();
        })
    });
    group.bench_function("gpu, full repaint (to completion)", |b| {
        b.iter(|| {
            ui.invalidate_all();
            let mut painter = gpu.painter(size);
            ui.paint_with(&mut Pen::new(&mut painter), BufferAge::Undefined);
            painter.finish(black_box(&view));
            gpu.device()
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("poll");
            ui.presented();
        })
    });
    // What damage buys on the GPU: the same pointer move as the software case,
    // repainting only what it dirtied onto a target kept between frames. The
    // same tree as above, deliberately — a second `Ui` would bring a second
    // glyph atlas and a second page upload, which is what the assertion at the
    // end of this function is for.
    let at = centre(&ui, buttons[7]);
    let elsewhere = Point::new(at.x, at.y + 400);
    let mut damage = [Rect::ZERO; 16];
    group.bench_function("gpu, damaged", |b| {
        b.iter(|| {
            ui.handle(&[InputEvent::PointerMoved { position: at }]);
            ui.handle(&[InputEvent::PointerMoved {
                position: elsewhere,
            }]);
            let count = {
                let pending = ui.pending_damage();
                let n = pending.len().min(damage.len());
                damage[..n].copy_from_slice(&pending[..n]);
                n
            };
            let mut painter = gpu.painter(size);
            ui.paint_with(&mut Pen::new(&mut painter), BufferAge::Frames(1));
            painter.finish_onto(black_box(&view), &damage[..count]);
            ui.presented();
        })
    });
    group.finish();

    // The atlas is the point of #187: a bench that uploaded a page per frame
    // would be measuring the wrong thing, and this says it did not.
    assert_eq!(
        gpu.page_uploads(),
        uploads,
        "the glyph page was re-uploaded during the bench"
    );
}

fn frames(c: &mut Criterion) {
    software(c, HD, "1920x1080");
    gpu(c, HD, "1920x1080");
    software(c, UHD, "3840x2160");
    gpu(c, UHD, "3840x2160");
}

criterion_group!(benches, frames);
criterion_main!(benches);
