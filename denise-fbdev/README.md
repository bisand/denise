# denise-fbdev

[![crates.io](https://img.shields.io/crates/v/denise-fbdev?color=CBA6F7&label=crates.io)](https://crates.io/crates/denise-fbdev)
[![docs.rs](https://img.shields.io/docsrs/denise-fbdev?color=94E2D5&label=docs.rs)](https://docs.rs/denise-fbdev)
[![Licence](https://img.shields.io/badge/licence-MIT-89B4FA)](https://github.com/bisand/denise/blob/main/LICENSE)

The legacy Linux fbdev backend for **[Denise]**, a direct-rendering UI toolkit in
Rust for embedded Linux and systems without a desktop environment.

## Read this before reaching for it

This is a **fallback**, and on any current kernel it is a fallback to DRM through a
longer route. `/dev/fb0` on a modern system is almost always
`CONFIG_DRM_FBDEV_EMULATION` — DRM pretending to be fbdev. An Alpine VM reports its
framebuffer's name as `virtio_gpudrmfb`; a Raspberry Pi running Bookworm reports
`vc4drmfb`. Going through it means giving up page flips, vsync and buffer age, to
reach the same hardware [`denise-drm`](https://crates.io/crates/denise-drm) already
drives properly.

So prefer DRM. Use this when:

- the kernel is old enough to predate a usable DRM driver for the panel,
- the panel has an fbdev driver and no DRM driver at all, which still happens with
  small SPI displays,
- or DRM master cannot be obtained and a degraded picture beats none.

```rust
# #[cfg(target_os = "linux")]
# fn demo() -> Result<(), Box<dyn std::error::Error>> {
use denise::Surface;
use denise_fbdev::FbdevSurface;

let mut surface = FbdevSurface::open_first()?;
println!("{:?}", surface.size());
# Ok(())
# }
```

## What it costs

No page flip and no vsync, so a frame can tear. Drawing goes through a shadow
buffer and only damaged rows are copied out, which keeps the tear as small as the
change that caused it — the only mitigation available here, and another reason
damage tracking belongs in the core rather than in a backend.

## Permissions

Writing to `/dev/fb*` needs the `video` group, or root.

## Platform

Linux only; elsewhere the crate compiles to almost nothing. `FbInfo` and the pixel
layout parsing are platform-independent and unit tested everywhere, because
misreading a `varinfo` bitfield is exactly the bug that shows as wrong colours on
one board and nowhere else. `unsafe` is permitted here and every block carries a
`// SAFETY:` comment.

## Where this sits

Implements `denise::Surface`. Pair it with
[`denise-evdev`](https://crates.io/crates/denise-evdev) for input.

## Status

**M2 complete.** Part of [Denise][Denise] — see the [repository README][Denise] for
the whole picture.

MIT licensed.

[Denise]: https://github.com/bisand/denise
