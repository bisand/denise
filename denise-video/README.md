# denise-video

[![crates.io](https://img.shields.io/crates/v/denise-video?color=CBA6F7&label=crates.io)](https://crates.io/crates/denise-video)
[![docs.rs](https://img.shields.io/docsrs/denise-video?color=94E2D5&label=docs.rs)](https://docs.rs/denise-video)
[![Licence](https://img.shields.io/badge/licence-MIT-89B4FA)](https://github.com/bisand/denise/blob/main/LICENSE)

Hardware video decode for **[Denise]**, a direct-rendering UI toolkit in Rust
for embedded Linux and systems without a desktop environment.

A kiosk playing a promo loop is a set-top box with a different sticker, and
this crate does what a set-top box does: compressed bytes go to the SoC's
decoder over **V4L2 memory-to-memory**, decoded frames come back as dmabufs,
and each dmabuf is flipped onto a **DRM plane** the display controller
composites during scanout. The frame never touches the rasteriser and is
never copied. No ffmpeg, no GStreamer, no C library — the V4L2 uapi is spoken
directly, the same single-static-binary discipline as `denise-drm`.

## The format menu

Two elementary streams, chosen so **every Raspberry Pi hardware-plays at least
one**: H.264 (`.h264`, Pi Zero–4 and most embedded SoCs) and HEVC (`.h265`,
Pi 4 and 5). Both yuv420, at most 1080p30. The board picks — a V4L2 decoder
*enumerates* what it accepts, so detection is a question, not a guess:

```rust
# #[cfg(target_os = "linux")] {
use denise_video::{Asset, Decoders};
let assets = [Asset::h264("promo.h264"), Asset::h265("promo.h265")];
if let Some((asset, node)) = Decoders::detect().pick(&assets) {
    println!("playing {} via {}", asset.path.display(), node.path.display());
}
# }
```

A kiosk ships both files; the asset pipeline is two ffmpeg lines at build time:

```text
ffmpeg -i in.mp4 -c:v libx264 -profile:v main -pix_fmt yuv420p -an -bsf:v h264_mp4toannexb out.h264
ffmpeg -i in.mp4 -c:v libx265 -pix_fmt yuv420p -an -bsf:v hevc_mp4toannexb out.h265
```

No container, no demuxer, no seeking — play, loop and stop, which is what a
promo loop is. Audio is a different subsystem and deliberately absent.

## Playing

`Player` drives the transport from the application's own event loop, against
the **same DRM card the surface owns** — one process is DRM master, and
`DrmSurface::card()` is the seam:

```text
let mut player = Player::open(&assets, surface.card(), surface.crtc(), rect)?;
loop {
    player.pump(surface.card())?;   // feeds, flips; never blocks
    // ... the UI keeps drawing itself; the plane composes over it
}
```

In the tree, the `Video` widget in `denise-ui` is the rectangle the plane
sits in: it reserves the space, paints the letterbox ground, and the
application hands its bounds to `Player::set_dst`. The frames never come
through the tree.

## Status

The **stateful** decoder path: `bcm2835-codec` on the Pi and its equivalents
on i.MX, Rockchip and Amlogic. The **stateless** HEVC path (`rpivid`, the
price of the Pi 5) is detected and reported but not yet driven — it is
tracked as its own issue, being an order of magnitude more work: slice
parsing, reference management, the media request API.

Linux only; on anything else the crate compiles to the pure Annex-B module
and nothing more. The `probe` example prints what a board can do, and is the
same enumeration `Decoders::detect` runs at runtime.

MIT licensed.

[Denise]: https://github.com/bisand/denise
