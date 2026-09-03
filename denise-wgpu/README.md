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

## What it does not do yet

Every `blit` still uploads its source as a fresh texture. Correct, and the next
thing a profile will point at: an image handle is the remaining piece, and like
the glyph page it is an addition to the painting trait rather than a change to
this crate alone.

The output is not byte-identical to the software rasteriser and is not meant to
be — the two anti-alias differently. The parity tests compare within a
tolerance, which is the honest test for a second backend.

[Denise]: https://github.com/bisand/denise
