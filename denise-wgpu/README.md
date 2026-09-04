# denise-wgpu

[![crates.io](https://img.shields.io/crates/v/denise-wgpu?color=CBA6F7&label=crates.io)](https://crates.io/crates/denise-wgpu)
[![docs.rs](https://img.shields.io/docsrs/denise-wgpu?color=94E2D5&label=docs.rs)](https://docs.rs/denise-wgpu)
[![Licence](https://img.shields.io/badge/licence-MIT-89B4FA)](https://github.com/bisand/denise/blob/main/LICENSE)

A GPU painter for **[Denise]**, on wgpu. The second implementation of the
painting trait, and the reason the trait exists: `denise-render` turns the same
calls into pixels on a CPU, this crate turns them into triangles on whatever wgpu
can find. Widgets cannot tell which one they are drawing through.

**This is for the desktop.** A kiosk on a Pi draws with the software rasteriser
and always will — no Mesa, no compositor, no window system, and at the sizes a
panel runs it was never the bottleneck. A GPU earns its keep where the designer
runs: a Retina display, a large window, a canvas that zooms.

```rust
use denise::{BufferAge, Pen, Size};
use denise_wgpu::Gpu;

# fn paint(ui: &mut denise_ui::Ui<()>) -> Result<(), denise_wgpu::Error> {
// Any adapter, no window: how tests and `--snapshot` paths draw.
let gpu = Gpu::headless()?;

let mut painter = gpu.painter(Size::new(640, 400));
ui.paint_with(&mut Pen::new(&mut painter), BufferAge::Undefined);

// `0xAARRGGBB` words, row after row — the layout a `denise::Frame` uses.
let pixels: Vec<u32> = painter.finish_to_pixels()?;
# Ok(())
# }
```

In a window, `Gpu::new` takes the device, queue and surface format you already
have, and `GpuPainter::finish` draws into the swapchain's texture view.
`examples/window.rs` is the whole of that, on winit.

## How a frame is drawn

Every call on the painter appends vertices. Rectangles and polygons are plain
triangles; everything with a curve — rounded corners, circles, arcs, lines — is
a bounding quad whose fragment shader evaluates a signed distance and turns it
into one pixel of anti-aliasing. Images and glyph masks are the same quads with
a texture bound. The clip is carried per vertex and applied per fragment, so a
frame is one pipeline and as few draws as the textures force: a widget tree
with no images is a single draw call.

## Glyphs

Text arrives through `Painter::blit_glyph` as a rectangle of an atlas page with
an id and a version. The page is uploaded once per version — that is, whenever
the text engine packs a glyph it has not seen — and every glyph after that is
six vertices sampling it. A frame of familiar text uploads nothing, and
`Gpu::page_uploads` is the number that says so.

## Images

Pictures arrive through `Painter::blit_image` with an id and a version, the
same way a glyph page does, and are cached the same way: one upload when the
pixels change, a quad every time after. `Gpu::image_uploads` counts them. A
photo in a carousel costs what a rectangle costs.

## What it costs, and where it wins

`cargo bench -p denise-benches --bench gpu`, on an M-series Mac, over the `ui`
bench's five-hundred-node HMI. Two sizes, because the interesting number is the
slope: the rasteriser's cost is per pixel, this crate's is per primitive.

| | 1920×1080 | 3840×2160 | scaling |
|---|---:|---:|---:|
| software, full repaint | 628 µs | 2 656 µs | ×4.2 |
| software, damage only | 6.6 µs | 5.4 µs | ×0.8 |
| GPU, full repaint (encode + submit) | 324 µs | 639 µs | ×2.0 |
| GPU, full repaint (to completion) | 790 µs | 1 284 µs | ×1.6 |
| GPU, damage only | 38 µs | 37 µs | **×0.98** |
| GPU, empty frame — submit throughput, not a floor | 48 µs | 48 µs | ×1.00 |

**On a full repaint the GPU wins, and by more the bigger the window gets** —
324 µs against 628 µs at 1080p, 639 µs against 2 656 µs at 4K. Four times the
area is 4.2× the work for the rasteriser and 2.0× here, which is the difference
between a cost that is per pixel and one that is mostly per primitive.

**Incremental repaint is free of resolution.** Damage on the GPU costs 38 µs at
1080p and 37 µs at 4K. What is left is fixed per-frame cost — a vertex buffer
built, a pass encoded, a submit — and it does not care how large the window is.
The globals buffer and its bind group are built on a resize rather than on a
frame, which is worth about 15% of that.

**Which is why the rasteriser still wins a small repaint.** 6.6 µs against
38 µs. Almost none of those 38 µs is drawing — it is a buffer built, an
encoder, a render pass and a submit, each a call into a driver, and each
costing the same whether eight pixels change or eight million. The rasteriser
writes into memory it already holds and hands nothing to anybody, so it has no
such cost at all.

The `empty frame` row was added to measure that floor directly and **does not
measure it**, which took a while to establish. It reports 48 µs — higher than a
real damaged frame's 38 µs, which cannot be a floor. The cause is the
benchmark: an undrained loop submits some thirty thousand frames a second, far
faster than the device retires them, so what it reports is how fast submits
*pile up*. A bind group shared by every queued command buffer costs the tracker
more than a throwaway one, and that shows up only in a loop nothing throttles.

Draining the queue each iteration settles it. Measured that way the two are the
same — 189 µs against 191 µs — while the real damaged frame keeps its
improvement, 196 µs against 230 µs. The same 15% the undrained numbers report.
So the change is real, the anomaly was the measurement, and the row is kept
under a name that says what it actually watches. The two costs cross where the damaged area is large enough for
the rasteriser's per-pixel work to reach the GPU's fixed 50 µs, and that is a
constant *area* rather than a constant fraction:

| | software per 1000 px | crossover |
|---|---:|---|
| 1920×1080 | 0.303 µs | 127 000 px — a 356×356 square, 6.1% of the screen |
| 3840×2160 | 0.320 µs | 117 000 px — a 342×342 square, 1.4% of the screen |

**So: change less than about a 350×350 square and the rasteriser is cheaper;
change more and this crate is, by a margin that grows with the window.** A
hovered button is the first case. A canvas being panned, a scrolling list, a
resize, anything animating across a Retina window is the second.

The `to completion` row is deliberately pessimistic: it makes the CPU wait for
the device, which no 60 Hz loop does, and it is a latency measurement rather
than a throughput one. `encode + submit` is what an application actually pays,
though not purely — it grows with resolution, so the driver is doing work
proportional to the attachment inside submit.

For scale, every row here fits inside a 60 Hz budget of 16 667 µs; the slowest
is 16% of a frame. None of this is the difference between working and not.

## What it does not do yet

The remaining fixed cost is not irreducible, but the obvious moves are not
free. Keeping the vertex buffer and writing into it with `write_buffer` was
tried and made things *worse* — the staging machinery costs more than the
allocation it saves at these sizes, in both a ring of three buffers and a
single one. The swapchain copy is still a second submit, and that has not been
tried. The `empty frame` anomaly, which used to be listed here, turned out to be the
benchmark and is settled.

`finish_onto` scissors to the *union* of the damage rather than rectangle by
rectangle, so two distant regions cost their bounding box in fragments. Doing
better means replaying the vertices once per region, which is not obviously a
win and has not been tried.

The raw `blit` family — a `PixelView` with no identity — still uploads per
call. Nothing in the widget set uses it any more; it is there for a caller
whose pixels genuinely are different every frame, which is what an upload per
call is the honest price of.

The output is not byte-identical to the software rasteriser and is not meant to
be — the two anti-alias differently. The parity tests compare within a
tolerance, which is the honest test for a second backend.

[Denise]: https://github.com/bisand/denise
