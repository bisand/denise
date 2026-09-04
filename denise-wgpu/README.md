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
| software, full repaint | 600 µs | 2 485 µs | ×4.1 |
| software, damage only | 6.3 µs | 5.2 µs | ×0.8 |
| GPU, full repaint (encode + submit) | 306 µs | 641 µs | ×2.1 |
| GPU, full repaint (to completion) | 777 µs | 1 315 µs | ×1.7 |

Read it in three parts.

**The slope is the point.** Four times the area is 4.1× the work for the
rasteriser and 1.7× for the GPU, which is the difference between a cost that is
per pixel and one that is mostly per primitive.

**The crossover sits above 1080p.** At 1920×1080 a full software repaint is
*faster* end to end than a GPU one — 600 µs against 777 µs. At 3840×2160 the
GPU wins by 1.9×, and costs 3.9× less CPU (641 µs against 2 485 µs) because the
fragments are the device's problem rather than a core's.

**Damage beats both, by two orders of magnitude.** A pointer moving between two
buttons dirties two small rectangles, and the rasteriser repaints just those in
about 6 µs — 124× cheaper than any full GPU frame. A swapchain remembers
nothing, so the GPU path repaints the window every time and cannot play this
game at all. For a panel that is idle most of the second, that is the whole
comparison, and it is why the kiosk path never grew a GPU.

So this crate earns its keep on a large window whose content genuinely changes
every frame — a designer canvas being panned or zoomed on a Retina display —
and loses badly on an idle one. Damage on the GPU path, by keeping a persistent
target and scissoring the redraw, is the change that would close that gap; it is
not written yet.

The `encode + submit` row is the CPU's share, but not purely: it grows with
resolution, so the driver is doing work proportional to the attachment inside
submit. Treat it as an upper bound on what a frame costs a core, not as a
measurement of recording alone.

## What it does not do yet

The raw `blit` family — a `PixelView` with no identity — still uploads per
call. Nothing in the widget set uses it any more; it is there for a caller
whose pixels genuinely are different every frame, which is what an upload per
call is the honest price of.

The output is not byte-identical to the software rasteriser and is not meant to
be — the two anti-alias differently. The parity tests compare within a
tolerance, which is the honest test for a second backend.

[Denise]: https://github.com/bisand/denise
