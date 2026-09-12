---
title: How it works
description: Surfaces and frames, damage tracking, the widget tree, scenes and overlays, scrolling, motion, what an idle panel costs, and the three text tiers.
---

One idea runs through the whole toolkit: **repaint what changed and nothing else, and make that the toolkit's job rather than the application's.** Most of what follows is that idea applied somewhere new. The long version, with the measurements and the things that were tried and abandoned, is [docs/design.md](https://github.com/bisand/denise/blob/main/docs/design.md).

## Surfaces and frames

The core crate, `denise`, contains no platform code. A backend implements two traits:

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

A `Frame` carries the pixel slice, its format, its **stride** and its **age**. The last two are why `acquire` exists rather than a bare buffer:

- **Stride is not width.** DRM framebuffers are pitch-aligned, so code assuming contiguous rows shears diagonally on the panel you shipped.
- **Buffers are stale.** With double buffering the buffer holds the frame *before* last, so repainting only this frame's damage leaves every second frame showing old content. `DamageTracker::resolve` widens the damage to cover what that buffer missed.

```rust
let mut frame = surface.acquire()?;
// The buffer may be several frames old; widen the damage to match.
let damage: &[Rect] = tracker.resolve(frame.age());
// ... draw, clipped to `damage` ...
drop(frame);
surface.present(damage)?;
tracker.end_frame();
```

An application on `denise-ui` does not write that sequence — `Ui::render` does — but it is part of the contract so no backend can get it wrong privately.

## Damage tracking

### No dirty flags

A stale colour on a Pi, from an application forgetting the one field that decided the pixels, is why this stopped being the application's job:

- `Ui::widget_mut` invalidates **on access**. Taking a mutable reference is the declaration that the widget will look different.
- Hover, press, focus and enabled are tracked by the tree, so a widget cannot forget a state it does not own.
- Moving, resizing, showing, hiding, adding or removing a node damages both the rectangle it left and the one it now occupies.

A test asserts that after any poke at the tree, an incremental repaint is **pixel-identical to a full repaint**.

### Repaint the field, not the window

The clip is the only damage-awareness the drawing code has: a widget paints as though it owned the whole window, and narrowing the clip to a damage rectangle turns that into an incremental repaint, so there is never a second draw path to keep in step. On DRM the cursor moves on the **hardware cursor plane** — one ioctl, no repaint.

`DamageTracker` never allocates, because it sits in the render hot path. It keeps up to `MAX_DAMAGE_RECTS` (16) rectangles per frame, collapsing to their bounding box past that, and remembers `MAX_TRACKED_FRAMES` (4) frames of history.

### What it buys depends on the backend

| Backend | Effect of `present(damage)` |
|---|---|
| Win32 `BitBlt`, AppKit | Real. Only the listed regions are copied. |
| DRM/KMS page flip | Little. A flip swaps whole buffers. |

On DRM the saving is upstream: not rasterising untouched pixels at all. On a Pi 3 A+ a full repaint of the benchmark scene costs 57.6% of a 60 Hz frame against the damaged version's 0.6%.

## The tree

Widgets live in a generational arena and the tree stores ids, so a stale `NodeId` resolves to `None` rather than to whoever was allocated next. Event handling returns messages, never callbacks. Siblings are sorted by z, making paint order a depth-first walk, and non-interactive widgets are invisible to hit testing — which is why a `Label` on top of a `Button` does not swallow the click.

Every node is a rectangle relative to its parent. **The tree never asks a widget how big it wants to be**; widgets with a natural size answer `Widget::measure` when *the application* asks. The placement rules the tree does own — anchors (`Ui::set_anchors`), docking (`Ui::set_dock`) and the vertical stack (`Ui::set_stack`) — each derive one rectangle per child in `reflow`, the single pass that turns layouts into bounds. Paint, damage, clipping and hit testing all read what that pass wrote, so they cannot disagree about where a node is.

## Scenes and overlays

A tree is one surface: on DRM there is no window system to open a second one in, and an embedded control that spawned a top-level window would escape its host's modality. So every layer lives in the same buffer:

| Layer | Call | Behaviour |
|---|---|---|
| Modal | `Ui::push_scene` | Another root over a dimmed backdrop. Input and Tab reach only the topmost scene. |
| Popup | `Ui::push_popup` | Anchored to a node, flipped when the surface runs out. A press outside closes it *and is swallowed*. Escape closes it; focus returns to the anchor. |
| Drawer | `Ui::push_drawer` | A scene that slides in. It pops when the exit slide lands. |
| Shelf | `Ui::push_shelf` | Slides in **without** pushing a scene, so the focused field keeps its caret. This is what the on-screen keyboard sits on. |
| Tooltip | `Ui::set_tooltip` | Not a node. The tree runs the dwell timer and draws it above everything. Needs hover, so it does nothing on a touch-only panel. |
| Toast | `Ui::toast` | Not a node. Stacked from the bottom, fades in and out, and costs one wake during its hold. |

Only the topmost veil paints, so a popup inside a modal does not darken the modal it serves. The one exception to one-surface-per-tree is the desktop backend: `denise-winit` runs one tree per window, so a settings form can be a real window where a window manager exists. A kiosk build never compiles it.

## Scrolling is a tree concern

Mark a node with `Ui::set_scrollable` and it becomes a viewport, its offset applied in the same `reflow`. The wheel scrolls the innermost viewport under the pointer after the hovered widget declines it; PageUp and PageDown page the one holding focus; a touch on its background drags it; and focusing something below the fold scrolls it into view. A scroll shifts the rows it keeps and paints only the new strip.

Two widgets scroll themselves, each for a structural reason rather than for taste. `Table`, because its header must stay pinned — and it windows its data, so rows outside the window are never iterated. `TextArea`, because what it scrolls is a document it does not hold: it cannot be a viewport over nodes that do not exist, so it keeps a first line and a sideways offset of its own, with a bar down the side and one along the bottom. The horizontal one measures the widest line it has *drawn*, because a document read a line at a time cannot be asked how wide the file is, and it is absent rather than greyed when the text fits.

Smooth and inertial scrolling are deliberately absent everywhere: a kiosk animating a fling at 60 Hz is the idle-cost story in reverse.

## Motion

A widget says *that* it is moving; the tree says *when*. `Wake::Animating` is a rate the tree owns and `Wake::At` a deadline it must not touch, so one setting governs every spinner, slide, layout tween and toast fade without changing any deadline:

```rust
ui.set_motion(Motion::Every(33));  // 30 fps: half the wakes, half the cost
ui.set_motion(Motion::None);       // reduced motion, or a tight power budget
```

The default is 16 ms. It is a sample rate, not a duration: a toggle still crosses in 120 ms and a carousel still advances after eight seconds at any setting. `Motion::None` lands transitions at their end state and leaves the tree asking for no wake at all. The setting lives on `Ui` rather than `Theme`, because swapping dark for light should not change the power budget. The gallery on a Pi 3 A+ over DRM, one spinner turning:

| Motion | CPU |
|---|---|
| 16 ms, the default | 4.20% |
| 33 ms | 2.06% |
| 50 ms | 1.26% |
| off | 0.00% |

`Spinner` is the only unbounded animation, and it does not start itself: the application calls `Ui::request_animation`.

## What an idle panel costs

On a Pi 3 A+ at 1920×1080, the `panel` demo left untouched for ten seconds — with a text field focused, so its caret blinks — draws **20 frames**, wakes **20 times** and spends **80 ms of CPU in total**, most of that on the two full repaints a double-buffered swapchain owes at startup. The loop blocks on the input descriptors and the deadline from `Ui::next_wake_ms`; with nothing focused there is no deadline and it blocks indefinitely.

## Text

Three tiers, chosen by feature so you pay for what you draw. The cost is the increase in a stripped, statically linked `aarch64-unknown-linux-musl` binary:

| Tier | Feature | Cost | Buys |
|---|---|---|---|
| Built-in bitmap | none | 0 | Latin plus `ÆØÅ æøå ÄÖÜ äöü`, whole-number scales |
| TrueType, via `fontdue` | `truetype` | +145 KB | Real faces, anti-aliased, any size |
| Shaped, via `cosmic-text` | `shaping` | +3.1 MB | Ligatures, bidirectional text, font fallback, complex scripts |

The shaping tier is four times the rest of the toolkit put together, and the choice is not obvious: on Arabic, `truetype` draws the right glyphs unjoined and in logical order — which looks like text, is wrong, and nobody who cannot read the script will notice.

Which face a widget uses is not the widget's decision. Every style names `FontId::DEFAULT`, a redirection that `Ui::set_default_font` points at a registered face. **No font ships with Denise**: a panel should carry the one face it was designed around.

## The rasteriser

`denise-render` draws into a `Frame` needing neither `std` nor `alloc`, containing no `unsafe`, and using **no floating point at all** — so output is bit-identical on x86 and ARM, and a pixel-exact test means the same thing on a laptop and on the Pi.
