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
| GPU, damage only | 51 µs | 54 µs | **×1.06** |
| GPU, empty frame — the floor | 31 µs | — | — |

**On a full repaint the GPU wins, and by more the bigger the window gets** —
324 µs against 628 µs at 1080p, 639 µs against 2 656 µs at 4K. Four times the
area is 4.2× the work for the rasteriser and 2.0× here, which is the difference
between a cost that is per pixel and one that is mostly per primitive.

**Incremental repaint is all but free of resolution.** Damage on the GPU costs
51 µs at 1080p and 54 µs at 4K — ×1.06 for four times the pixels. What is left
is fixed per-frame cost: two buffers built, a pass encoded, two submits, a
present. It does not care how large the window is.

**Which is why the rasteriser still wins a small repaint.** An *empty* GPU
frame — a painter made, nothing recorded, one 8×8 damage submitted — costs
31 µs. That is the floor, and it is already five times what the rasteriser
spends doing the entire job. None of it is drawing: it is two buffers built, a
bind group, an encoder, a render pass and a submit, each a call into a driver.
The rasteriser writes into memory it already holds and hands nothing to
anybody, so it has no such floor at all. The two costs cross where the damaged area is large enough for
the rasteriser's per-pixel work to reach the GPU's fixed 50 µs, and that is a
constant *area* rather than a constant fraction:

| | software per 1000 px | crossover |
|---|---:|---|
| 1920×1080 | 0.303 µs | 168 000 px — a 410×410 square, 8.1% of the screen |
| 3840×2160 | 0.320 µs | 169 000 px — a 411×411 square, 2.0% of the screen |

**So: change less than about a 410×410 square and the rasteriser is cheaper;
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

The 31 µs floor is not irreducible. A globals buffer and a
vertex buffer are built from scratch every frame, the swapchain copy is a
second submit, and neither has to be that way. That is where a small repaint
would have to get cheaper for this crate to win one.

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
