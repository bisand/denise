# denise-drm

[![crates.io](https://img.shields.io/crates/v/denise-drm?color=CBA6F7&label=crates.io)](https://crates.io/crates/denise-drm)
[![docs.rs](https://img.shields.io/docsrs/denise-drm?color=94E2D5&label=docs.rs)](https://docs.rs/denise-drm)
[![Licence](https://img.shields.io/badge/licence-MIT-89B4FA)](https://github.com/bisand/denise/blob/main/LICENSE)

The Linux DRM/KMS backend for **[Denise]** — and the primary target the whole
toolkit exists for.

Opens the display directly, sets a mode, and page-flips CPU-rendered dumb buffers
straight to the scanout engine. **No compositor, no window server, no GPU driver
stack, no X.** One static binary that owns the panel.

```rust
# #[cfg(target_os = "linux")]
# fn demo() -> Result<(), denise_drm::DrmError> {
use denise::Surface;
use denise_drm::{DrmSurface, PresentMode, SurfaceConfig};

// Opens the first card with a display output, takes master, picks a mode and
// builds the swapchain.
let mut surface = DrmSurface::open(SurfaceConfig {
    present_mode: PresentMode::Vsync,
    ..SurfaceConfig::default()
})?;
println!("mode {} at {:?}", surface.mode_name(), surface.size());
# Ok(())
# }
```

`DrmSurface::new(card, config)` takes a `Card` you opened yourself — including one
handed over as a file descriptor, which is how this coexists with `libseat` or a
systemd unit.

## Becoming DRM master

Setting a mode requires being DRM master, and only one process can be. If a
compositor or another Denise process holds it, `Card::become_master` fails with
`EBUSY` or `EACCES` — **the single most common reason a first run on a Pi does
nothing.** Three ways to have the right:

- Run on a bare VT with no display server. The usual kiosk deployment.
- Be handed a file descriptor by `libseat` or a systemd unit, via `Card::from_fd`.
  Preferred for anything that has to coexist.
- Run as root. Works, and is a poor way to ship a product.

## What it deliberately does not do

**No `gbm`.** GBM exists to allocate buffers a *GPU* renders into. Denise renders
with the CPU, so DRM dumb buffers are exactly the right allocation: scanout-capable,
CPU-mappable, and free of any C library. Adding GBM would drag in libgbm and Mesa,
ending both the single-static-binary goal and easy cross-compilation.

**No atomic modesetting, yet.** Atomic buys `FB_DAMAGE_CLIPS`, plane composition
and tear-free guarantees. The first is worth little here — a page flip swaps whole
buffers, so damage saves rasterisation, not bandwidth, and most drivers ignore the
property. The second has a legacy equivalent for the one plane that matters, the
hardware cursor. So the legacy path gets this working on real hardware at a third
of the code, behind a seam atomic can take over when planes earn their keep.

## When a frame tears, and when it waits

`PresentMode::Immediate` flips without waiting for vblank, because on a Pi 3A+ at
1920×1080 that wait is about 17 ms and it is felt as lag by the person pressing
the button. A button redrawing itself damages a few thousand pixels, so the seam
where the panel switched buffers mid-scan is small, brief and invisible.

A scrolling viewport is the other case, and it fails twice over. The seam crosses
the text being read, and — because an async flip never blocks — nothing paces the
loop, so the same Pi spends a whole core producing torn frames at 14.5 ms each
for as long as a finger keeps moving.

So the flip follows the damage rather than the setting alone: **under a quarter
of the surface it is async, at or above it waits for vblank.** The latency stays
where it is felt, the pacing arrives on the frames that cannot afford to go
without it, and an application does not have to know which kind of frame it just
drew. `PresentMode::Vsync` is still an absolute promise; it is `Immediate` that
became a preference.

## Testing

Mode selection, the swapchain and the flip decision are platform-independent *on
purpose* and are unit tested everywhere, including on machines with no DRM
device. They hold the
decisions that are hard to debug in the field and easy to check on a laptop.
Everything else is a thin wrapper over ioctls and can only be proven on real
hardware — which it has been, on a Raspberry Pi.

Two examples help on a new board: `probe` prints the connectors, modes and
capabilities it finds, and `smoke` sets a mode and flips.

## Platform

Linux only. On any other target the crate compiles to almost nothing, which is what
lets the whole workspace be checked and published from one runner. `unsafe` is
permitted here and every block carries a `// SAFETY:` comment.

## Where this sits

Implements `denise::Surface`. Pair it with
[`denise-evdev`](https://crates.io/crates/denise-evdev) for input — both expose raw
file descriptors, so an event loop can `epoll` on input and vblank together and the
process idles in the kernel rather than spinning.
[`denise-fbdev`](https://crates.io/crates/denise-fbdev) is the fallback for kernels
with no usable DRM driver.

## Status

**M2 complete**, running on real hardware. Part of [Denise][Denise] — see the
[repository README][Denise] and [docs/design.md] for the whole picture.

MIT licensed.

[Denise]: https://github.com/bisand/denise
[docs/design.md]: https://github.com/bisand/denise/blob/main/docs/design.md
