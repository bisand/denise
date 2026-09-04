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

Every call on the painter appends vertices. Rectangles are plain triangles;
everything with a curve — rounded corners, circles, arcs, lines — is a bounding
quad whose fragment shader evaluates a signed distance and turns it into one
pixel of anti-aliasing. A polygon has more edges than a vertex can carry, so its
quad carries a range of a buffer that holds them, and the shader walks that
range for the nearest edge and for which side of the outline the fragment is on
— by the even-odd rule, the same one the rasteriser fills by. Images and glyph
masks are the same quads with a texture bound. The clip is carried per vertex and applied per fragment, so a
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

`cargo bench -p denise-benches --bench gpu_cpu`, on an M-series Mac, over the
`ui` bench's five-hundred-node HMI. **Process CPU time per frame** — user plus
system, every thread — with one drain inside the measured window so work the
driver deferred is still counted.

CPU time, not wall time, and the distinction turned out to matter more than
anything else measured here. This path runs threads of its own: at 1080p a
damaged frame occupies 139 µs of CPU across 85 µs of wall clock, which is 163%
of one core. Every wall-clock figure understates it, and the earlier ones in
this file did.

| | 1920×1080 | 3840×2160 |
|---|---:|---:|
| software, full repaint | 692 µs | 3 311 µs |
| software, damage only | **7.2 µs** | **6.3 µs** |
| GPU, full repaint | **496 µs** | **781 µs** |
| GPU, damage only | 139 µs | 145 µs |
| GPU, empty frame | 80 µs | 83 µs |

**On a full repaint the GPU wins, by 1.4× at 1080p and 4.2× at 4K.** Four times
the area is 4.8× the work for the rasteriser and 1.6× here, which is the
difference between a cost that is per pixel and one that is mostly per frame.

**On an incremental repaint it loses badly — 139 µs against 7.2 µs, nineteen
times.** An *empty* GPU frame already costs 80 µs of CPU: a buffer built, a
pass encoded, a submit, and whatever the driver does on its own threads, none
of it drawing and none of it caring how much changed. The rasteriser writes
into memory it already holds and hands nothing to anybody, so it has no such
cost.

The two cross where the damaged area is large enough for the rasteriser's
per-pixel work to reach the GPU's per-frame cost — a constant *area*, not a
constant fraction:

| | software per 1000 px | crossover |
|---|---:|---|
| 1920×1080 | 0.334 µs | 417 000 px — a 646×646 square, 20% of the screen |
| 3840×2160 | 0.399 µs | 362 000 px — a 602×602 square, 4.4% of the screen |

**So: change less than about a 600×600 square and the rasteriser is cheaper;
change more and this crate is, by a margin that grows with the window.** A
hovered button, a caret, a label is the first case. A canvas being panned, a
scrolling list, a resize, a full-window animation is the second.

The wall-clock benches beside this one (`--bench gpu`) are kept for what they
do measure — submit throughput, and round-trip latency — but neither is a
per-frame cost, and #193 is the story of finding that out the hard way.

## What it does not do yet

The 80 µs an empty frame costs is not irreducible, but the obvious moves are
not free. Keeping the vertex buffer and writing into it with `write_buffer` was
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
