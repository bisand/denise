//! What a frame costs a **core**, rather than how long it takes.
//!
//! The wall-clock benches next door cannot answer that, and #193 is why. Left
//! undrained, a submit loop reports how fast frames pile up; drained, it is
//! floored by a synchronisation round trip that costs the same whether the
//! frame drew five hundred widgets or nothing at all. Neither is the number
//! that decides whether an application has headroom.
//!
//! So this measures process CPU time — user plus system, every thread — across
//! a run of frames, with **one drain inside the measured window**. Draining
//! once at the end means work the driver deferred to its own threads is still
//! counted, while no frame pays the round trip that dominates a per-frame
//! drain. Wall time is reported beside it: where the two diverge, the
//! difference is time spent waiting rather than working.
//!
//! Not a criterion bench. Criterion measures wall time, and wall time is the
//! thing that misled us.
//!
//! ```text
//! cargo bench -p denise-benches --bench gpu_cpu
//! ```

use std::time::{Duration, Instant};

use denise::{BufferAge, Frame, InputEvent, Pen, PixelFormat, Point, Rect, Size, theme};
use denise_ui::widgets::{Button, Label, Panel};
use denise_ui::{NodeId, Ui};
use denise_wgpu::{Gpu, wgpu};

const PANELS_X: i32 = 5;
const PANELS_Y: i32 = 4;
const CONTROLS: i32 = 12;
const HD: Size = Size::new(1920, 1080);
const UHD: Size = Size::new(3840, 2160);

/// How long each case runs before it is believed.
const TARGET: Duration = Duration::from_millis(600);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Msg {
    Pressed(u32),
}

/// Process CPU time: user plus system, across every thread this process has.
///
/// `RUSAGE_SELF` rather than a thread clock on purpose — wgpu and the driver do
/// work on threads of their own, and an application pays for that as surely as
/// for what it does itself.
fn cpu_time() -> Duration {
    // SAFETY: `getrusage` writes a `rusage` this call owns and reads nothing
    // else; the zeroed value is a valid initial state for it.
    let usage = unsafe {
        let mut usage = std::mem::zeroed::<libc::rusage>();
        assert_eq!(
            libc::getrusage(libc::RUSAGE_SELF, &mut usage),
            0,
            "getrusage failed"
        );
        usage
    };
    let secs =
        |t: libc::timeval| Duration::new(t.tv_sec as u64, (t.tv_usec as u32).saturating_mul(1_000));
    secs(usage.ru_utime) + secs(usage.ru_stime)
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
                let (row, column) = (i / 2, i % 2);
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
    let b = ui.bounds(id).expect("bounds");
    Point::new(b.x + b.width / 2, b.y + b.height / 2)
}

/// Runs `frame` until [`TARGET`] elapses, then `settle` once, all inside one
/// CPU-time window. Reports both costs per frame.
fn measure(name: &str, mut frame: impl FnMut(), settle: impl FnOnce()) {
    // Warm up: first frames pack the glyph atlas and build pipelines.
    for _ in 0..20 {
        frame();
    }

    let (cpu0, wall0) = (cpu_time(), Instant::now());
    let mut runs = 0u32;
    while wall0.elapsed() < TARGET {
        frame();
        runs += 1;
    }
    // Inside the window: whatever the driver deferred is still this run's cost.
    settle();
    let (cpu, wall) = (cpu_time() - cpu0, wall0.elapsed());

    let per = |d: Duration| d.as_secs_f64() * 1e6 / f64::from(runs);
    println!(
        "  {name:<34} cpu {:>8.1} µs   wall {:>8.1} µs   {:>5.0}% working",
        per(cpu),
        per(wall),
        100.0 * per(cpu) / per(wall).max(f64::EPSILON),
    );
}

fn suite(size: Size, label: &str) {
    println!("\n{label}");

    // ---- the rasteriser: nothing deferred, nothing to settle -------------
    {
        let (mut ui, buttons) = busy_panel(size);
        let mut pixels = vec![0u32; (size.width * size.height) as usize];
        let at = centre(&ui, buttons[7]);
        let elsewhere = Point::new(at.x, at.y + 400);

        measure(
            "software, full repaint",
            || {
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
            },
            || {},
        );
        measure(
            "software, damaged",
            || {
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
            },
            || {},
        );
    }

    // ---- the GPU: settle once, so deferred work is counted ---------------
    let gpu = match Gpu::headless() {
        Ok(gpu) => gpu,
        Err(err) => {
            println!("  (skipping the GPU cases: {err})");
            return;
        }
    };
    let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("cpu-cost target"),
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
    let drain = || {
        gpu.device()
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll");
    };

    let (mut ui, buttons) = busy_panel(size);
    {
        let mut painter = gpu.painter(size);
        ui.paint_with(&mut Pen::new(&mut painter), BufferAge::Undefined);
        painter.finish(&view);
        drain();
        ui.presented();
    }
    let at = centre(&ui, buttons[7]);
    let elsewhere = Point::new(at.x, at.y + 400);
    let mut damage = [Rect::ZERO; 16];

    measure(
        "gpu, full repaint",
        || {
            ui.invalidate_all();
            let mut painter = gpu.painter(size);
            ui.paint_with(&mut Pen::new(&mut painter), BufferAge::Undefined);
            painter.finish(&view);
            ui.presented();
        },
        drain,
    );
    measure(
        "gpu, damaged",
        || {
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
            painter.finish_onto(&view, &damage[..count]);
            ui.presented();
        },
        drain,
    );
    let speck = [Rect::new(0, 0, 8, 8)];
    measure(
        "gpu, empty frame",
        || {
            let painter = gpu.painter(size);
            painter.finish_onto(&view, &speck);
        },
        drain,
    );
}

fn main() {
    println!(
        "Process CPU time per frame (user + system, every thread), with one \
         drain inside\nthe measured window. Wall time beside it; the percentage \
         is how much of the\nwall was spent working rather than waiting."
    );
    suite(HD, "1920x1080");
    suite(UHD, "3840x2160");
}
