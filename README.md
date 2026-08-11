<div align="center">

<img src="assets/logo.svg" width="148"
     alt="Three translucent layers compositing into one, with a cursor sprite on top">

# Denise

**A direct-rendering UI toolkit for embedded Linux and systems without a desktop
environment.**

[![CI](https://github.com/bisand/denise/actions/workflows/ci.yml/badge.svg)](https://github.com/bisand/denise/actions/workflows/ci.yml)
[![Licence](https://img.shields.io/badge/licence-MIT-89B4FA)](LICENSE)
[![MSRV](https://img.shields.io/badge/MSRV-1.95-F9E2AF)](#constraints)
[![Milestone](https://img.shields.io/badge/milestone-M3-F5C2E7)](#milestones)
[![Core](https://img.shields.io/badge/core-forbid(unsafe__code)-A6E3A1)](#constraints)
[![Targets](https://img.shields.io/badge/targets-aarch64_%7C_armv7_%7C_x86__64-94E2D5)](#constraints)

</div>

Kiosks, digital signage, industrial HMIs, Raspberry Pi panels, in-vehicle displays.

No X11. No Wayland. No browser engine. No managed runtime. One static binary that
opens the display, draws, and reads input.

Denise is a from-scratch Rust successor to
[CoreCanvas](https://github.com/bisand/corecanvas), a .NET library that proved the
architecture: scene stack, z-index layering, dirty-rectangle tracking, composite
cursor sprite, 60 FPS at under 5% CPU on a Pi 4. Denise keeps the design and drops
the runtime.

## Status: M3

Denise drives a real display with no desktop environment, and now has a user
interface to put on it: a retained widget tree in a generational arena, a scene
stack with modal dialogs over a dimmed backdrop, a composite cursor sprite, and
Label, Button and TextInput. Underneath, DRM/KMS scanout with async page flips,
an fbdev fallback, evdev input, damage tracking and theming — verified on a
Raspberry Pi 3 A+ driving a 1920×1080 output with no X.

<img src="assets/showcase.png" width="620"
     alt="A panel of themed buttons, text fields and a modal dialog over a dimmed backdrop">

Every pixel above was rendered by `cargo run -p denise-ui --example showcase`,
which writes a PPM and needs no display at all.

On a Raspberry Pi, read [docs/raspberry-pi.md](docs/raspberry-pi.md) first — a
stock Pi has no `/dev/dri` at all until the vc4 KMS overlay is enabled, and that
one line decides whether you get real page flips or a tearing firmware
framebuffer.

On a Linux machine with a spare VT or a virtual GPU:

```bash
cargo run -p panel            # the widget tree, a modal dialog, no X
cargo run -p kiosk            # M2's instrumented loop: latency percentiles
cargo run -p denise-drm --example probe   # read-only: what it would drive
```

The number that matters is what it costs when nothing happens. Left untouched for
three seconds, the kiosk demo draws **one frame**, wakes **thirteen times**, and
uses no measurable CPU — it blocks in `poll` on the input descriptors rather than
spinning. Move the pointer and it repaints a cursor-sized rectangle, not a
megapixel.

Without a display, the desktop preview backend runs the same scene code unchanged:

```bash
cargo run -p hello-rect
```

## The name

Denise — the MOS 8362, one of the three custom chips in the Amiga's original chip
set alongside Agnus and Paula — was the display chip. Its job, every single frame:

1. pull bitplanes out of chip RAM and turn them into pixels,
2. combine two playfields according to priority,
3. overlay up to eight hardware sprites,
4. resolve it all through the colour registers,
5. hand the result to the video output.

That is this library's job description, done in software: composite z-ordered
layers, overlay a cursor sprite, resolve to pixels, hand the buffer to the display.
The cursor sprite is not a metaphor — it is step 4 of the
[rendering pipeline](#rendering-pipeline) below, and it is why there is one in the
logo.

There is a second reason the name fits, and it is the one that actually decided it.
Denise did all of the above with no operating system in the way. No compositor, no
window server, no driver stack, no GPU. Chip RAM to display, on a fixed cycle
budget, on hardware slower than the microcontroller in a modern keyboard. That is
precisely the position this toolkit wants to occupy on a Raspberry Pi, and
precisely the thing X11 and Wayland exist to stop you doing.

Super Denise (8373) extended it in ECS; AGA replaced it with Lisa.

### What the backends are not called

Agnus, Paula and Lisa are not backend crate names, and will not become them. They
are excellent names for a blog post and useless in a repository: `denise-drm` tells
you what it does, `denise-paula` requires you to already know. The Amiga reference
stops at the project name.

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
| `denise-render` | Software rasteriser and the built-in bitmap font | ✅ M1, M3 |
| `denise-ui` | Scene graph, scene stack, widgets, cursor sprite | ✅ M3 |
| `denise-winit` | Desktop development and preview backend | ✅ M0 |
| `denise-drm` | Linux DRM/KMS backend — the primary target | ✅ M2 |
| `denise-fbdev` | Linux fbdev fallback | ✅ M2 |
| `denise-evdev` | Linux input | ✅ M2 |
| `denise-text` | Font loading, glyph cache, shaping | M4 |
| `denise-ffi` | Stable C ABI, `cdylib` | M5 |
| `denise-win32` | Windows child-HWND control | M5 |
| `denise-macos` | Layer-backed `NSView` | M5 |
| `denise-activex` | COM/ActiveX shim for legacy Windows hosts | M5 |

Everything through M3 exists. The rest are listed so the shape of the thing is clear.

`denise-ui` is a crate of its own rather than part of the core because widgets
need both the platform contract and the rasteriser, and the rasteriser already
depends on the contract — putting them together would be a dependency cycle. It
also means a signage application that draws its own scene links no arena, no tree
and no widget code at all.

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

Ported from CoreCanvas, and what `Ui::paint` does, in this order, inside each
damage rectangle:

1. Clear the back buffer — clean UI, no cursor.
2. Render the base scene.
3. Render modal scenes over a dimmed backdrop.
4. Composite the cursor sprite onto the clean buffer.
5. Present damaged rectangles only.

Every step happens inside the damage clip, which is what makes step 3 affordable:
a full-screen alpha fill costs 63% of a 60 Hz frame on a Pi 3, so a modal that
repaints its own blinking caret must not drag one along with it.

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

## The component model

Widgets live in a generational arena; the tree stores ids, not references. Event
handling returns messages, not callbacks. No `Rc<RefCell<_>>` anywhere in the
path.

```rust
let mut ui: Ui<Msg> = Ui::new(surface.size(), theme::DARK);
let card = ui.add(ui.root(), Panel::default(), Rect::new(40, 40, 400, 260))?;
ui.add(card, Button::new("Lagre", Msg::Save), Rect::new(20, 180, 160, 46))?;

ui.handle(&events);
ui.render(&mut surface)?;              // draws nothing when nothing changed
for message in ui.drain_messages() { /* the application decides */ }
```

A stale `NodeId` resolves to `None` rather than to whoever was allocated next,
which is the entire reason for the generation in the key.

### Damage is the toolkit's job

This is the part that changed as a result of M2. A bug on the Pi produced a
stale colour on a card because the application's idea of "what changed" left out
the one field that decided the pixels. The fix is not care; it is that the
application no longer has that job:

- `Ui::widget_mut` invalidates **on access**, before you have changed anything.
  Taking `&mut` to a widget is the declaration that it will look different.
- Hover, press, focus and enabled are tracked by the tree, so a widget cannot
  forget to invalidate a state it does not own.
- Moving, resizing, showing, hiding, adding or removing a node damages both the
  rectangles it vacated and the ones it now occupies.

[`denise-ui/tests/scene.rs`](denise-ui/tests/scene.rs) asserts the property that
subsumes all of it: after any poke at the tree, an incremental repaint through a
double-buffered swapchain with a padded stride is **pixel-identical to a full
repaint**. A missed invalidation fails there rather than three weeks later on
somebody's panel.

### Scenes, not dialog widgets

A modal is a scene pushed on the stack, not a widget inside the page. Input goes
to the topmost scene only, so nothing underneath is hittable, focusable or
reachable by Tab — a property of the stack rather than something each dialog has
to enforce. The backdrop is painted per damage region, never over the whole
surface.

### Hit testing and paint order

Siblings are kept sorted by z as they are added, so flattening the tree is a
plain depth-first walk, cached and rebuilt only on a structural or z-order
change. Non-interactive widgets are invisible to hit testing, which is why a
`Label` inside a `Button` does not swallow the click.

### Numbers, at five hundred nodes

`cargo bench -p denise-benches --bench ui`, on an Apple M-series at 1920×1080
with a 500-node tree — twenty panels of controls, which is a busy HMI:

| Benchmark | Time |
|---|---|
| Hit test, topmost node | 4.0 ns |
| Hit test, deepest node | 297 ns |
| Hit test, miss (walks everything) | 265 ns |
| Rebuild the paint order after a z change | 754 ns |
| Full repaint | 497 µs |
| One button hovered, then unhovered | 5.3 µs per frame |
| One button pressed, then released | 4.1 µs per frame |

A frame that changed one button costs **about a hundredth of a percent** of a
full repaint of the same tree. That ratio is the whole design, measured against
the size the architecture was specified for.

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

### The built-in font

Five by seven in an eight-row cell, monospace, integer-scaled: printable ASCII
plus `ÆØÅ æøå ÄÖÜ äöü Éé ß °` and a few units and dashes. Anything else draws a
visible box rather than nothing.

This is M4's promised built-in font brought forward, because a Label, a Button and
a TextInput without glyphs are three rectangles. It is deliberately not
extensible — no font loading here, and none coming. Glyphs are ASCII art in the
source, packed into bits by a `const fn` at compile time, because a table of hex
bytes cannot be reviewed and a picture of a `Ø` can. `cargo run -p denise-render
--example fontdump -- "Kjøre på Æ"` prints them without a display.

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
  no platform crates — `denise`, `denise-render` and `denise-ui` all held to it.
  An embedded build that quietly starts compiling winit is a regression.
- MSRV 1.95, bumped deliberately rather than tracking stable.

## Milestones

| | | |
|---|---|---|
| **M0** | Workspace, `Surface`/`InputSource`, winit backend, damage tracking, CI | ✅ |
| **M1** | Software rasteriser: rects, rounded rects, lines, clipping, alpha blend. Benches. | ✅ |
| **M1.1** | Theming: semantic colour roles, guaranteed-contrast content pairing, geometry tokens. | ✅ |
| **M2** | DRM/KMS with legacy modesetting and page flip; fbdev fallback; evdev input. Runs with no X. | ✅ |
| **M3** | Scene stack, z-index, modal dialogs, cursor sprite. Label, Button, TextInput. CoreCanvas 0.4 parity. | ✅ |
| **M4** | Text: `cosmic-text` behind a feature flag with a glyph atlas, proportional metrics, shaping. The built-in bitmap font shipped early with M3. | |
| **M5** | C ABI, Windows child-HWND control, ActiveX shim, macOS `NSView`. | |

M2 does not start until M1 is benchmarked. M5 does not start until the Pi story is
solid — that is the entire point of the project.

M2 shipped legacy modesetting rather than atomic, which reverses the original
plan. What atomic buys is `FB_DAMAGE_CLIPS`, plane composition and tear-free
guarantees; a page flip swaps whole buffers, so damage saves rasterisation rather
than bandwidth, and the one plane worth having — the hardware cursor — has a
legacy equivalent. Atomic slots in behind the same seam when planes earn it.

M3 pulled the built-in bitmap font forward from M4. The milestone that ships
Label, Button and TextInput needs glyphs, and shipping three rectangles instead
would have been a milestone in name only. M4 still owns real text: `cosmic-text`,
an atlas, proportional metrics and shaping.

Still outstanding, and deliberately not hidden:

- **The VT keyboard is not muted** while Denise holds DRM master, so keystrokes
  still reach the shell behind the UI. Real kiosks mute it with `KDSKBMODE`/`K_OFF`.
- **Touch is unverified on hardware.** The multitouch slot path is unit tested and
  a single touch is routed to widgets as a pointer would be, but no physical
  touchscreen has driven it.
- **No text selection, clipboard or word motion** in `TextInput`. Half of that is
  meaningless without a font that can measure a substring, which is M4.
- **No layout engine.** Nodes are positioned with explicit rectangles relative to
  their parent, which is what a fixed-resolution panel wants; a constraint solver
  can be added over this without changing anything below it.

## Development

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p hello-rect
cargo run -p denise-ui --example showcase -- dark showcase.ppm   # no display needed
```

## Licence

MIT — see [LICENSE](LICENSE).
