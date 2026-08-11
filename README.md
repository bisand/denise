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

## Status: M1.1

The core abstraction, the software rasteriser and the theme system exist, benchmarked, and proven
against a desktop backend. There is no scene graph, no component model, no
hardware backend and no text yet. See [Milestones](#milestones).

What works today:

```bash
cargo run -p hello-rect
```

A rounded rectangle bounces around a window at 60 FPS while repainting roughly 4%
of the surface per frame. Press `T` to cycle themes; the drawing code does not
change, because it never names a colour. Stats print to stderr once a second —
that number is the whole point of the project, so it is measured from the first
commit rather than asserted in a README later.

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
| `denise` | Geometry, colour, pixel buffer contract, input, damage tracking, theming | ✅ M0, M1.1 |
| `denise-render` | Software rasteriser | ✅ M1 |
| `denise-winit` | Desktop development and preview backend | ✅ M0 |
| `denise-drm` | Linux DRM/KMS backend — the primary target | M2 |
| `denise-fbdev` | Linux fbdev fallback | M2 |
| `denise-evdev` | Linux input | M2 |
| `denise-text` | Font loading, glyph cache, layout | M4 |
| `denise-ffi` | Stable C ABI, `cdylib` | M5 |
| `denise-win32` | Windows child-HWND control | M5 |
| `denise-macos` | Layer-backed `NSView` | M5 |
| `denise-activex` | COM/ActiveX shim for legacy Windows hosts | M5 |

Only the first three exist. The rest are listed so the shape of the thing is clear.

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
[`denise-render/tests/damage_pipeline.rs`](denise-render/tests/damage_pipeline.rs),
which runs an anti-aliased scene through 1-, 2-, 3- and 6-buffered swapchains with
a padded stride and asserts every presented frame is pixel-identical to a full
repaint.

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

## The rasteriser

`denise-render` draws rectangles, rounded rectangles, lines and source-over alpha
straight into a `Frame`. It needs neither `std` nor `alloc`, contains no `unsafe`,
and uses **no floating point at all** — anti-aliasing coverage included.

That last one is a deliberate trade. Integer coverage means no `libm` on `no_std`
targets, no FPU traffic where that costs, and output that is bit-identical between
x86 and ARM — which is what makes a pixel-exact reference test meaningful on a
developer's laptop *and* on the Pi. Rounded corners are evaluated analytically per
scanline with an integer square root, at four sub-rows per scanline.

The clip is the only damage-awareness the drawing code has. Widget code paints as
though it owned the whole window; restricting the clip to a damage region turns
that into an incremental repaint, so there is never a second draw path to keep in
step with the first.

### Numbers

Apple M-series, `--release`, so read the *ratios*, not the absolute times — a Pi 4
is an order of magnitude slower on memory-bound work.

| Benchmark | Time |
|---|---|
| 1080p scene, full repaint | 211 µs |
| 1080p scene, typical damage (0.4% of the surface) | **5.7 µs** |
| 1080p blit, whole buffer | 101 µs |
| 1080p blit, same damage | **0.88 µs** |
| 800×480 scene, full repaint | 54 µs |
| Rounded rect fill 1600×900, r=8 / r=32 | 77 µs / 78 µs |

A damaged frame costs **37× less** than a full one, and the rounded-rect cost is
flat in the radius — anti-aliasing is paid per perimeter pixel, not per area. Both
are the properties the design was aiming at.

Two results worth keeping in view:

- **A padded stride costs 3× on a full clear** (59 µs → 185 µs at 1080p). DRM hands
  out pitch-aligned buffers, so the padded number is the one that will matter on
  hardware. Whether that gap is stride handling or simply the larger buffer falling
  out of cache is not yet established.
- **`fill_rect` currently measures slower than `fill_rounded_rect` on the same
  rectangle**, which cannot be right — the rounded path does strictly more work.
  Unexplained, and flagged rather than papered over.

```bash
cargo bench --workspace
```

CI compiles the benches but does not gate on their timings: wall-clock variance on
a shared runner is far wider than any threshold worth setting. The regression gate
belongs on a self-hosted Pi, or on instruction counts.

## Theming

The role vocabulary is borrowed from [daisyUI](https://daisyui.com), which got the
important part right: a widget never names a colour, it names a **role**, and every
surface role has a **content** partner. Swapping a theme cannot produce unreadable
text, because readability is a property of the pair rather than of the widget.

```rust
let (background, foreground) = theme.pair(Role::Primary);
let corner = theme.radius(Radius::Box);
```

Twenty roles — `base-100/200/300` plus `base-content`, then `primary`,
`secondary`, `accent`, `neutral`, `info`, `success`, `warning` and `error`, each
with a content partner. Three radius tokens by widget class (`Selector`, `Field`,
`Box`) rather than one constant per widget, which is what stops the set drifting.

A theme is built from nine seed colours; the two recessed base surfaces and all
nine content colours are derived by walking towards black or white until the mix
clears **WCAG 4.5:1**, so a derived theme keeps its hue instead of collapsing to
black on white. `Theme::from_seeds` is a `const fn`, so the built-in themes cost
nothing at runtime and cannot drift out of step with the derivation rules.

`Theme::validate` checks every pair, and it earns its place: it caught that pure
magenta and `#FF5555` both top out near 6.7:1 against black and cannot reach AAA,
which is why the high-contrast palette uses lightened variants.

Three themes ship — `LIGHT`, `DARK`, and `HIGH_CONTRAST` for panels read in glare
or through a visor. On a device booting from flash, an unused theme is bytes
somebody paid for.

### What was not borrowed

| | |
|---|---|
| **OKLCH storage** | Cube roots mean floats, which mean `libm` on `no_std` and output that is no longer bit-identical across architectures. Colours are sRGB, derived with integers. |
| **`--noise`** | A per-pixel texture makes every pixel differ from its neighbour, so no damaged region can be repainted without a seam against the region beside it. It turns every frame into a full repaint. |
| **35 built-in themes** | Three. |
| **`--depth` as a shadow** | Kept as a number. A real blur is expensive in software and spills outside the widget's bounds, so every damage rectangle would have to be inflated by the blur radius. |

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
| **M1** | Software rasteriser: rects, rounded rects, lines, clipping, alpha blend. Benches. | ✅ |
| **M1.1** | Theming: semantic colour roles, guaranteed-contrast content pairing, geometry tokens. | ✅ |
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
