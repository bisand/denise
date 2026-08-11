//! Scene-tree costs: hit testing, flattening, and what a frame costs.
//!
//! Five hundred nodes is the size the bootstrap asked for, and it is a fair
//! caricature of a busy HMI — a dozen panels of controls. The number that decides
//! whether the design works is the last group: a tree this size must cost almost
//! nothing to repaint when almost nothing changed.

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use denise::{
    BufferAge, ElementState, Frame, InputEvent, Modifiers, PixelFormat, Point, PointerButton, Rect,
    Size, theme,
};
use denise_ui::widgets::{Button, Label, Panel};
use denise_ui::{NodeId, Ui};

const SIZE: Size = Size::new(1920, 1080);
/// Panels across and down, and controls in each: 20 panels of 24 controls plus
/// their labels comes to a little over five hundred nodes.
const PANELS_X: i32 = 5;
const PANELS_Y: i32 = 4;
const CONTROLS: i32 = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Msg {
    Pressed(u32),
}

/// Builds the tree, and returns the id of one button near the far end of the
/// paint order — the worst case for a hit test that walks topmost-first.
fn busy_panel() -> (Ui<Msg>, Vec<NodeId>) {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let mut buttons = Vec::new();
    let panel_w = SIZE.width as i32 / PANELS_X;
    let panel_h = SIZE.height as i32 / PANELS_Y;

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

/// Centre of a node's bounds, for aiming a hit test at it.
fn centre(ui: &Ui<Msg>, id: NodeId) -> Point {
    let bounds = ui.bounds(id).expect("bounds");
    Point::new(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2)
}

fn hit_test(c: &mut Criterion) {
    let (mut ui, buttons) = busy_panel();
    // Warm the paint-order cache, so this measures traversal rather than the
    // flatten that only happens on a structural change.
    ui.hit_test(Point::ZERO);

    let first = centre(&ui, buttons[0]);
    let last = centre(&ui, *buttons.last().expect("buttons"));

    let mut group = c.benchmark_group("hit_test_500_nodes");
    group.bench_function("hit, last in paint order", |b| {
        b.iter(|| black_box(ui.hit_test(black_box(last))))
    });
    group.bench_function("hit, first in paint order", |b| {
        b.iter(|| black_box(ui.hit_test(black_box(first))))
    });
    group.bench_function("miss", |b| {
        // Nothing is there, so the whole order is walked: the worst case.
        b.iter(|| black_box(ui.hit_test(black_box(Point::new(1, 1)))))
    });
    group.finish();
}

fn flatten(c: &mut Criterion) {
    let (mut ui, buttons) = busy_panel();
    let target = buttons[0];
    c.bench_function("flatten_500_nodes_after_z_change", |b| {
        b.iter(|| {
            // Changing a z invalidates the cached paint order; the next hit test
            // pays for the rebuild. This is the cost of *not* caching it.
            ui.set_z(target, black_box(1));
            ui.set_z(target, black_box(0));
            black_box(ui.hit_test(Point::new(1, 1)))
        })
    });
}

fn frames(c: &mut Criterion) {
    let (mut ui, buttons) = busy_panel();
    let mut pixels = vec![0u32; (SIZE.width * SIZE.height) as usize];

    let mut paint = |ui: &mut Ui<Msg>, age: BufferAge| {
        let mut frame =
            Frame::new(&mut pixels, SIZE, SIZE.width, PixelFormat::Xrgb8888, age).expect("frame");
        ui.paint(&mut frame);
        drop(frame);
        ui.presented();
    };

    let mut group = c.benchmark_group("frame_500_nodes");
    group.bench_function("full repaint", |b| {
        b.iter(|| {
            ui.invalidate_all();
            paint(&mut ui, BufferAge::Undefined);
        })
    });

    let at = centre(&ui, buttons[7]);
    let elsewhere = Point::new(at.x, at.y + 400);
    group.bench_function("hover moved between two buttons", |b| {
        b.iter(|| {
            ui.handle(&[InputEvent::PointerMoved {
                position: black_box(at),
            }]);
            paint(&mut ui, BufferAge::Frames(2));
            ui.handle(&[InputEvent::PointerMoved {
                position: black_box(elsewhere),
            }]);
            paint(&mut ui, BufferAge::Frames(2));
        })
    });

    group.bench_function("one button pressed and released", |b| {
        b.iter(|| {
            ui.handle(&[InputEvent::PointerButton {
                button: PointerButton::Left,
                state: ElementState::Down,
                position: at,
                modifiers: Modifiers::NONE,
            }]);
            paint(&mut ui, BufferAge::Frames(2));
            ui.handle(&[InputEvent::PointerButton {
                button: PointerButton::Left,
                state: ElementState::Up,
                position: at,
                modifiers: Modifiers::NONE,
            }]);
            paint(&mut ui, BufferAge::Frames(2));
            ui.drain_messages().for_each(drop);
        })
    });
    group.finish();
}

criterion_group!(benches, hit_test, flatten, frames);
criterion_main!(benches);
