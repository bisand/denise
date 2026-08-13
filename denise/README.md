# denise

[![crates.io](https://img.shields.io/crates/v/denise?color=CBA6F7&label=crates.io)](https://crates.io/crates/denise)
[![docs.rs](https://img.shields.io/docsrs/denise?color=94E2D5&label=docs.rs)](https://docs.rs/denise)
[![Licence](https://img.shields.io/badge/licence-MIT-89B4FA)](https://github.com/bisand/denise/blob/main/LICENSE)

The platform-agnostic core of **[Denise]** — a direct-rendering UI toolkit in Rust
for embedded Linux and systems without a desktop environment: kiosks, digital
signage, industrial HMIs, Raspberry Pi panels.

This crate is the contract every other one is written against: geometry, colour,
the pixel buffer, input events, damage tracking, theming and the cursor sprite.
It contains **no platform code**, `#![forbid(unsafe_code)]`, and builds
`no_std + alloc`. Backends are separate crates chosen at build time, so an
embedded target never compiles desktop code.

## The two traits a backend implements

```rust
use denise::{DamageTracker, Rect, Surface};

# fn demo(surface: &mut impl Surface, tracker: &mut DamageTracker) -> Result<(), denise::SurfaceError> {
let mut frame = surface.acquire()?;
// The buffer just acquired may be several frames old; widen the damage to match.
let damage: &[Rect] = tracker.resolve(frame.age());
// ... draw, clipped to `damage` ...
drop(frame);
surface.present(damage)?;
tracker.end_frame();
# Ok(())
# }
```

`Surface` hands out a `Frame` and presents damaged regions; `InputSource` drains
platform input into `InputEvent`s. Getting the sequence above wrong is the classic
double-buffering bug — repaint only the current frame's damage and every second
frame shows stale content, which is why `BufferAge` is part of the contract rather
than a backend's private business.

## Theming, and why contrast is checked

Colours are named by **role**, never by value: `Primary`, `Base100`, `Error`, and a
`*Content` partner for each. `Theme::pair` returns a surface and a foreground that
are *guaranteed* to contrast, and `Theme::validate` proves it for a custom theme
rather than leaving it to be discovered on a panel in daylight. The role vocabulary
is borrowed from DaisyUI; the contrast arithmetic is WCAG relative luminance in
integers, so it is identical on x86 and ARM.

## Features

| Feature | Default | What it does |
|---|:---:|---|
| `std` | ✅ | `std::error::Error` for the error types. Turn it off for `no_std + alloc`. |

## Where this sits

| Crate | Role |
|---|---|
| **`denise`** | This crate — the contract |
| [`denise-render`](https://crates.io/crates/denise-render) | Software rasteriser writing into a `Frame` |
| [`denise-ui`](https://crates.io/crates/denise-ui) | Scene graph, widgets, compositor |
| [`denise-drm`](https://crates.io/crates/denise-drm) · [`denise-winit`](https://crates.io/crates/denise-winit) · [`denise-fbdev`](https://crates.io/crates/denise-fbdev) | `Surface` implementations |
| [`denise-evdev`](https://crates.io/crates/denise-evdev) | `InputSource` implementation |

## Status

**M5 complete, M6 in progress.** Everything through M5 has run on real hardware —
a Raspberry Pi on DRM/KMS, Windows 11 on ARM64, macOS — not only in CI. The
roadmap, the design notes and what was tried and abandoned are in the
[repository README][Denise] and [docs/design.md].

MIT licensed.

[Denise]: https://github.com/bisand/denise
[docs/design.md]: https://github.com/bisand/denise/blob/main/docs/design.md
