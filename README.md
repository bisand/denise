# Denise

A direct-rendering UI toolkit for embedded Linux and systems **without a desktop
environment** — kiosks, digital signage, industrial HMIs, Raspberry Pi panels,
in-vehicle displays.

No X11. No Wayland. No browser engine. No managed runtime. One static binary that
opens the display, draws, and reads input.

The name is the Amiga's display chip, which composited bitplanes and hardware
sprites into a video signal. That is more or less the job description, cursor
sprite included.

Denise is a from-scratch Rust successor to
[CoreCanvas](https://github.com/bisand/corecanvas), a .NET library that proved the
architecture: scene stack, z-index layering, dirty-rectangle tracking, composite
cursor sprite, 60 FPS at under 5% CPU on a Pi 4. Denise keeps the design and drops
the runtime.

## Status: M0

The core abstraction exists and is proven against a desktop backend. There is no
scene graph, no component model, no hardware backend and no text yet. See
[Milestones](#milestones).

What works today:

```bash
cargo run -p hello-rect
```

A rectangle bounces around a window at 60 FPS while repainting roughly 4% of the
surface per frame. Stats print to stderr once a second — that number is the whole
point of the project, so it is measured from the first commit rather than asserted
in a README later.

## What it is not

- A general desktop application framework. If you want one, use
  [egui](https://github.com/emilk/egui), [Iced](https://iced.rs) or
  [Slint](https://slint.dev); they are good and this is not competing with them.
- HTML, CSS or a WebView.
- A visual designer or a markup language. Perhaps much later.
- Fully accessible, or capable of complex-script text shaping, in 0.x. The
  architecture leaves room for both. Neither is delivered.

## Architecture

A Cargo workspace: a platform-agnostic core, and thin backends behind two traits.

| Crate | Purpose | Status |
|---|---|---|
| `denise` | Geometry, colour, pixel buffer contract, input, damage tracking | M0 |
| `denise-winit` | Desktop development and preview backend | M0 |
| `denise-render` | Software rasteriser | M1 |
| `denise-drm` | Linux DRM/KMS backend — the primary target | M2 |
| `denise-fbdev` | Linux fbdev fallback | M2 |
| `denise-evdev` | Linux input | M2 |
| `denise-text` | Font loading, glyph cache, layout | M4 |
| `denise-ffi` | Stable C ABI, `cdylib` | M5 |
| `denise-win32` | Windows child-HWND control | M5 |
| `denise-macos` | Layer-backed `NSView` | M5 |
| `denise-activex` | COM/ActiveX shim for legacy Windows hosts | M5 |

Only the first two exist. The rest are listed so the shape of the thing is clear.

### The two traits

Everything a backend has to provide:

```rust
pub trait Surface {
    fn size(&self) -> Size;
    fn scale_factor(&self) -> f32;
    fn format(&self) -> PixelFormat;
    fn acquire(&mut self) -> Result<Frame<'_>, SurfaceError>;
    fn present(&mut self, damage: &[Rect]) -> Result<(), SurfaceError>;
}

pub trait InputSource {
    fn poll(&mut self, out: &mut Vec<InputEvent>);
}
```

`Frame` carries the pixel slice, its **stride**, its format, and its **age**. Those
last two are the whole reason `acquire` exists rather than a bare
`buffer_mut() -> &mut [u32]`:

- **Stride is not width.** DRM framebuffers are pitch-aligned — 64 bytes on vc4,
  more on other ARM drivers — and fbdev has its own `line_length`. Code that
  assumes rows are contiguous works perfectly on a desktop and shears diagonally on
  the panel you actually shipped.
- **Buffers are stale.** With double buffering the buffer you are handed holds the
  frame *before* last. Repainting only this frame's damage leaves the older
  content visible in alternating frames. `BufferAge` is modelled on
  `EGL_EXT_buffer_age`, and `DamageTracker::resolve` widens this frame's damage to
  cover everything that buffer missed.

Both failure modes are covered by
[`denise/tests/damage_pipeline.rs`](denise/tests/damage_pipeline.rs), which runs a
scene through 1-, 2-, 3- and 6-buffered swapchains with a padded stride and asserts
every presented frame is pixel-identical to a full repaint.

### Rendering pipeline

Ported from CoreCanvas, and the target for M3:

1. Clear the back buffer — clean UI, no cursor.
2. Render the base scene.
3. Render modal scenes over a dimmed backdrop.
4. Composite the cursor sprite onto the clean buffer.
5. Present damaged rectangles only.

On DRM, step 4 should use the hardware cursor plane rather than compositing into
the buffer; vc4 has one, and it makes pointer movement cost no redraw at all. The
software composite stays as the fallback.

### What damage actually buys

Worth being precise about, because it differs per backend:

| Backend | Effect of `present(damage)` |
|---|---|
| Win32 `BitBlt`, X11, Wayland | Real. Only the listed regions are uploaded. |
| DRM/KMS page flip | Little to none. A flip swaps whole buffers; `FB_DAMAGE_CLIPS` is atomic-only and widely ignored. |

On DRM the win is entirely upstream: not rasterising the untouched pixels in the
first place. That is where the CPU goes, so that is where the tracking pays.

## Constraints

- `unsafe_code = "forbid"` in `denise`. `unsafe` is allowed in backend crates only,
  every block carrying a `// SAFETY:` comment.
- Zero allocation in the render hot path. `DamageTracker` is fixed-capacity;
  coalescing degrades to a bounding box rather than allocating.
- The core builds `no_std + alloc`. `--no-default-features` is checked in CI.
- CI cross-compiles the core to `aarch64-unknown-linux-gnu` and
  `armv7-unknown-linux-gnueabihf`, and asserts the core's dependency tree contains
  no platform crates. An embedded build that quietly starts compiling winit is a
  regression.
- MSRV 1.95, bumped deliberately rather than tracking stable.

## Milestones

| | | |
|---|---|---|
| **M0** | Workspace, `Surface`/`InputSource`, winit backend, damage tracking, CI | ✅ |
| **M1** | Software rasteriser: rects, rounded rects, lines, clipping, alpha blend. Benches. | |
| **M2** | DRM/KMS with atomic modesetting and page flip; fbdev fallback; evdev input. Runs on a Pi with no X. | |
| **M3** | Scene stack, z-index, modal dialogs, cursor sprite. Label, Button, TextInput. CoreCanvas 0.4 parity. | |
| **M4** | Text: built-in 8×8 bitmap font; `cosmic-text` behind a feature flag with a glyph atlas. Latin plus `æøå`, dead keys included. | |
| **M5** | C ABI, Windows child-HWND control, ActiveX shim, macOS `NSView`. | |

M2 does not start until M1 is benchmarked. M5 does not start until the Pi story is
solid — that is the entire point of the project.

## Development

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p hello-rect
```

## Licence

MIT — see [LICENSE](LICENSE).
