<div align="center">

<img src="assets/logo.svg" width="148"
     alt="Three translucent layers compositing into one, with a cursor sprite on top">

# Denise

**A direct-rendering UI toolkit for embedded Linux and systems without a desktop
environment.**

[![CI](https://github.com/bisand/denise/actions/workflows/ci.yml/badge.svg)](https://github.com/bisand/denise/actions/workflows/ci.yml)
[![Licence](https://img.shields.io/badge/licence-MIT-89B4FA)](LICENSE)
[![MSRV](https://img.shields.io/badge/MSRV-1.95-F9E2AF)](#constraints)
[![Milestone](https://img.shields.io/badge/milestone-M5-F5C2E7)](#milestones)
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

## Status: M5

Denise drives a real display with no desktop environment, has a user interface to
put on it, text that types `æøå` including dead keys, and — since M5 — a way to
embed all of it in somebody else's application. Underneath, DRM/KMS scanout with
async page flips, an fbdev fallback, evdev input, damage tracking and theming —
verified on a Raspberry Pi 3 A+ driving a 1920×1080 output with no X.

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

The number that matters is what it costs when nothing happens. On a Raspberry Pi
3 A+ at 1920×1080, the panel demo left untouched for ten seconds — with a text
field focused, so its caret is blinking — draws **20 frames**, wakes **20 times**,
and spends **80 ms of CPU in total**, most of it on the two full repaints every
double-buffered swapchain owes at startup. It blocks in `poll` on the input
descriptors and the caret deadline rather than spinning; with nothing focused
there is no deadline either and it blocks indefinitely.

Move the pointer and it repaints two cursor-sized rectangles, not a megapixel.

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
| `denise-render` | Software rasteriser, coverage blitting, the built-in bitmap font | ✅ M1, M3, M4 |
| `denise-text` | Glyph sources, a bounded glyph atlas, line layout | ✅ M4 |
| `denise-ui` | Scene graph, scene stack, widgets, cursor sprite | ✅ M3 |
| `denise-winit` | Desktop development and preview backend | ✅ M0 |
| `denise-drm` | Linux DRM/KMS backend — the primary target | ✅ M2 |
| `denise-fbdev` | Linux fbdev fallback | ✅ M2 |
| `denise-evdev` | Linux input, keyboard layouts, dead keys, console muting | ✅ M2, M4, M5 |
| `denise-ffi` | Stable C ABI, `cdylib`, hand-written header | ✅ M5 |
| `denise-macos` | Embeddable `NSView` over a CoreGraphics bitmap context | ✅ M5 |
| `denise-win32` | Windows child-`HWND` control over a DIB section | ✅ M5 |
| `denise-activex` | COM/ActiveX shim for legacy Windows hosts, scriptable over `IDispatch` | ✅ M5 |

Everything through M5 has run on real hardware. On Windows 11 ARM64, `denise-win32`
puts a window on screen where Tab reaches the control, AltGr composes `@` and the
dead keys produce `é` and `ö`; and `denise-activex` registers with `regsvr32`,
instantiates through `CoCreateInstance`, sites, activates in place and renders
inside a container that knows nothing about it — which then sets its properties by
name over `IDispatch`, sinks its `Change` and `Click` events, and assigns back to
it from inside its own event handlers.

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

On DRM, step 4 uses the **hardware cursor plane** instead. vc4 has one, and the
display controller composites it during scanout, so moving the pointer is a single
ioctl — no repaint, no page flip, and the new position takes effect at the next
scanout of those lines rather than the next frame. `CursorPlane` in the core is the
seam; `Ui::show_cursor(false)` tells the tree to stop drawing its own, and the
software composite stays as the fallback for every backend without a plane.

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

Five by seven in an eight-row cell, monospace, integer-scaled. Glyphs are ASCII
art in the source, packed into bits by a `const fn` at compile time, because a
table of hex bytes cannot be reviewed and a picture of a `Ø` can. It is the first
of the three [text tiers](#text), and the only one that needs no feature flag, no
file and no allocator.

```bash
cargo run -p denise-render --example fontdump -- "Kjøre på Æ"
```

Anti-aliased glyphs from the other tiers arrive as 8-bit coverage masks. The
blitter walks each row in runs: solid runs — the inside of a glyph — go through
the span blend, empty runs are skipped, and the per-pixel path is paid only on the
rim. That matters because the M1 benches put the per-pixel path at 31 Mpx/s
against 457 for spans on a Pi 3, and predicted glyphs would be where the gap got
paid.

```bash
cargo bench --workspace
```

CI compiles the benches but does not gate on their timings: wall-clock variance on
a shared runner is far wider than any threshold worth setting. The regression gate
belongs on a self-hosted Pi, or on instruction counts.

## Text

Three tiers, chosen by what a panel actually has to draw. The cost column is the
increase in a stripped, statically linked `aarch64-unknown-linux-musl` binary,
measured rather than estimated.

| Tier | Feature | Cost | Buys |
|---|---|---|---|
| Built-in bitmap | none | 0 | Latin plus `ÆØÅ æøå ÄÖÜ äöü Éé ß °`, whole-number scales |
| TrueType | `truetype` | **+145 KB** | Real faces, anti-aliased, proportionally spaced, any size |
| Shaped | `shaping` | **+3.1 MB** | Ligatures, bidirectional text, font fallback, complex scripts |

For scale, the whole of Denise, DRM, evdev and the widgets is **848 KB**, so the
shaping tier is four times the rest of the toolkit put together.

The numbers that decide between them are stark in both directions. On a Norwegian
pangram, `truetype` and `shaping` produce lines **two pixels** different in total
width — three megabytes for two pixels. On Arabic they are not comparable at all:

- the built-in font draws **boxes** — obviously missing, obviously a defect;
- `truetype` draws **the right glyphs, unjoined and in logical order**, which is
  fluent nonsense: it looks like text, it is wrong, and nobody who cannot read the
  script will notice;
- only `shaping` joins them and runs them right to left.

That middle result is the one to be careful about. `examples/specimen` takes a
sample string as its third argument precisely so it can be checked before a device
ships:

```bash
cargo run -p denise-text --features truetype,shaping \
  --example specimen -- specimen.ppm MyFont.ttf "sample text"
```

**No font ships with Denise, and none will.** Type designers' licences differ, and
embedding somebody's typeface in a toolkit is a decision for whoever ships the
device. There is also no font discovery: nothing is read from a system font
directory, because a device that boots from flash with a read-only root very often
has none, and a UI whose text depends on that is a UI that fails in the field.
`cosmic-text`'s own `new_with_fonts` turned out to load the host's fonts anyway —
812 of them on the machine this was written on — which is why the database is
built by hand.

### The glyph cache

One buffer of coverage bytes, shelf-packed, with a size fixed at construction:
64 KB by default. A panel with a twenty-year service life wants "the glyph cache
is exactly 64 KB", not "however many glyphs the user has typed since Tuesday".

When it fills it resets wholesale rather than freeing rectangles, and counts the
reset — so a cache that is genuinely too small shows up as a rising number rather
than as a mystery. Measurement goes through it as well as drawing, which is what
stops a label being re-outlined on every layout pass.

| Benchmark | Time |
|---|---|
| Cache hit | 2.8 ns |
| Cache miss, 16 px | 100 ns |
| Cache miss, 24 px | 213 ns |
| Measure a 16-character label | 252 ns |
| Draw a 16-character label, 16 px | 1.39 µs |
| Draw the same label, 48 px | 6.91 µs |

## Keyboards

`KeyCode` names a *position*; what it types is a property of the user's layout.
`denise-evdev` ships US and Norwegian as static tables, with dead-key composition,
AltGr as a third level, and a Caps Lock that reaches `æøå` without turning `1`
into `!`.

The layout is read from the system: `DENISE_KEYMAP`, then `XKB_DEFAULT_LAYOUT`,
then the console keyboard configuration files distributions actually write. On the
Pi this was developed against, `/etc/conf.d/loadkmap` says Norwegian and the panel
picks it up with nothing set by hand.

```console
$ /tmp/panel
keymap  no (from /etc/conf.d/loadkmap)
```

The composition table is generated from Unicode's own canonical composition data
rather than typed out — a hand-written table of a hundred accented letters is a
list of a hundred chances to be subtly wrong about one of them.

### Why the tables are ours, when the choice is the system's

Reading which layout a system wants is easy. Reading the layout *itself* is the
part that would remove these tables, and both ways of doing it cost more than they
save:

| | What it gives | What it costs |
|---|---|---|
| `KDGKBENT` on a VT | The kernel's real keymap, dead-key table included | `/dev/tty0` is `root:root` mode 600 everywhere checked |
| libxkbcommon | Every layout in xkeyboard-config | A C library and a runtime data directory |

Denise otherwise runs unprivileged, needing only the `video` and `input` groups,
and a static binary needs no data directory. Giving up either to read a keymap is
a poor trade. So the choice comes from the system and the data comes from here.

The cost is real and stated: a system configured for a layout Denise has no table
for falls back to US, **visibly**, through the reported source — rather than by
typing the wrong thing. Adding a table is about thirty lines, because a layout
lists only what differs from the Latin alphabet. Needing root is forever.

Control characters are never text. Enter, Tab and Backspace produce `Key` events
and nothing else, so a field can insert everything it receives without filtering
and a key binding cannot be shadowed by a stray control character.

### Muting the console

Reading evdev does not stop anyone *else* reading it. On a console-booted kiosk
the login shell behind the UI receives every keystroke as well, so typing into a
Denise text field also types at the shell — and a form field that happens to
contain `reboot` followed by Enter does what it says. Holding DRM master stops the
console drawing; it does nothing about the keyboard.

`Console::mute_keyboard` sets `KDSKBMODE` to `K_OFF`. evdev sits below the console
layer, so Denise still sees everything and the shell sees nothing. It is paired
with `KDSETMODE`/`KD_GRAPHICS`, which stops console blanking on an idle panel and
stops the kernel repainting text after an oops.

Two things make this safe to ship rather than a footgun:

- **The guard restores on drop**, including while a panic unwinds, and it puts
  back the mode it *read* rather than a guess at the default.
- **A pty is refused.** `/dev/tty` over SSH is not a console, and `KDGKBTYPE` is
  the ioctl that says so. Without that check, `open` would hand back the first
  thing that opened and the developer's own terminal would be the one muted.

`K_OFF` also swallows `Ctrl+Alt+F2`, so a muted console cannot be escaped from at
the keyboard, and nothing restores it after `SIGKILL`. The escape hatch, over SSH,
is `kbd_mode -u -C /dev/tty1`.

## Embedding

M5 is the other direction. Everything before it is Denise owning the display; this
is Denise owning one rectangle inside an application that already exists — an MFC
dialog, a Cocoa window, a C or C# or Python host.

The shape is the same in all of them, and it is the one thing worth getting right:
**the host owns the window, the event loop and the pixel buffer; Denise owns the
widget tree and draws into whatever it is handed.** There is no `run` function in
any of these backends, and no `Surface` in the C ABI. A library that owned either
would be unembeddable in exactly the places this exists for.

| | Backing store | Present | Verified |
|---|---|---|---|
| `denise-ffi` | the caller's, described by `DeniseFrame` | the caller's problem | C and C++ example built and run in CI |
| `denise-macos` | `CGBitmapContext`, CoreGraphics owns the pixels | `setNeedsDisplayInRect:` then `CGContextDrawImage` | rendered through AppKit's own `cacheDisplayInRect:` |
| `denise-win32` | 32-bit top-down DIB section | `InvalidateRect` then `BitBlt` | compiles, unit tests pass on a Windows runner |

Three things fell out of doing it three times:

- **Damage means different things.** On DRM a page flip swaps whole buffers, so
  damage saves rasterisation and no bandwidth. On Win32 and on AppKit it saves
  both — `BitBlt` moves only what it is given. The same rectangles, worth
  measurably more.
- **Row zero is not agreed on.** A `CGImage` is bottom-up; a DIB section is
  bottom-up unless you ask for a negative height; Denise's row zero is the top.
  Neither platform reports a mistake here. It renders upside down and looks like
  somebody laid the widgets out wrong.
- **There is already a cursor.** Both hosts draw one, so the composited sprite has
  to stay off — and it did not, because the tree revealed it on every pointer move.
  `Ui::show_cursor` is now a decision that sticks.

The Windows CI job caught the first thing it was pointed at: two virtual key codes
naming the same position, because the unsided `VK_CONTROL` deliberately aliases
`VK_LCONTROL` and the test excluded only the shift case. The mapping was right; the
test's exclusion was too narrow. The fix worth having was not the one-line one —
`denise-win32`'s keymap is now platform-independent and its tests run everywhere,
the same split `denise-drm` and `denise-evdev` already made. A table of a hundred
numbers is exactly the thing that breaks, and a CI runner is a slow place to find
out.

### Scripting the ActiveX control

Embedding a control and *driving* one are different problems, and the second is
`IDispatch`. The surface is four members and two events:

| Member | Dispid | |
|---|---|---|
| `Text` | 1 | property, read/write — the field's contents |
| `Caption` | 2 | property, read/write — the heading |
| `Enabled` | 3 | property, read/write |
| `Refresh` | 4 | method |
| `Change` | 1 | event — somebody typed |
| `Click` | -600 | event — the button, at OLE's standard `DISPID_CLICK` |

```vbscript
Set p = CreateObject("Denise.Panel")
p.Caption = "Hei"
```

There is no type library, so a host is late-bound: it asks for a name and invokes
it. VBScript, JScript, VB6 through an `Object` variable, MFC's
`COleDispatchDriver` and every OLE container work that way and need nothing else.

**PowerShell is the exception, and chasing it was the most instructive part of
this.** It builds its member table from `ITypeInfo` and will not ask for a name it
has not been told about, so `$panel.Caption` fails with "cannot be found on this
object" before a single COM call is made — nothing is wrong with the control, it
has simply never been asked anything.

`CreateDispTypeInfo` looked like a cheap way out: hand it a method table and it
builds an `ITypeInfo` in memory, no `.tlb`, no `LIBID`, nothing to keep in step.
Two rounds of that produced two better errors and no fix. The first was mine —
every put claimed to return `VT_EMPTY`, which is a variant that *holds* nothing
rather than a call that *returns* nothing, and PowerShell duly unwrapped a null.
The second was the API's: `CreateDispTypeInfo` builds a vtable-shaped description,
`TKIND_INTERFACE` and not `TKIND_DISPATCH`, so PowerShell looked for a
dispinterface, did not find one, and produced an object with no members and no
complaint at all. Nothing in the method table changes the kind.

So it was removed. `GetTypeInfoCount` answers zero, which is honest, and PowerShell
reaches the control through `[System.__ComObject].InvokeMember` — which goes
straight to `Invoke` and works. The real fix is a registered type library, and it
buys a form designer's property sheet and early binding at the same time; it is on
the outstanding list rather than half-built.

The lesson worth keeping is about what the diagnostics cost. Each round was a
rebuild on a VM, a screenshot, and a guess about a COM adapter that cannot be run
from the machine the code is written on. What ended it was not a better guess but
making the object describe itself and printing that — at which point the answer was
one word wide.

Two more things are worth naming. The first is that **a host is not tidy about
`wFlags`** — VBScript sends `METHOD | PROPERTYGET` for anything whose result it
uses, because at the call site it does not know which the object has. So the flags
are a set of things the host would accept, and the control picks the one the member
offers. That decision is a pure function of a table, so it lives outside
`cfg(windows)` with tests, next to the HIMETRIC arithmetic and for the same reason.

The second is **re-entrancy**, which is where a control like this actually breaks.
A click handler assigning to `Caption` is an ordinary thing for a script to do, and
it arrives while the tree that raised the click is still running — with the
control's own `RefCell` borrowed around it. Pushing it straight back in would
panic, unwinding out through a COM method into somebody's script engine. So a
property put made while the tree is running records the change and stops, and the
tree applies whatever a handler left behind in a second pass before it returns. One
extra pass, deliberately not a loop: a handler that assigns on every event would
otherwise never hand control back.

On the ARM64 machine that is a few hundred round trips — a `Change` handler reading
`Text` on every keystroke of a sentence, and eighteen clicks each assigning
`Caption` — with the borrow held around all of them.

### The header is the contract

`denise-ffi`'s header is written by hand and the Rust is checked against it, not
generated from it. A generated header follows whatever the implementation says this
week, which is the opposite of what a stable ABI means.

[`tests/header.rs`](denise-ffi/tests/header.rs) does the checking, and it earns its
keep on the parts a linker cannot. A missing declaration is a link error the first
time anybody tries. A key number that differs between the two sides is not: the
host presses Enter, the field receives Home, and nothing anywhere says so.

The numbering is not arbitrary either. A key position is *named* after the US
layout, so positions carrying an ASCII character there are numbered with it —
`DENISE_KEY_A` is `0x41`, `DENISE_KEY_SEMICOLON` is `0x3B`. Half the table needs no
lookup and a key log is readable in hex.

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
- The core builds `no_std + alloc` — `denise`, `denise-render`, `denise-text` and
  `denise-ui` all of them. `--no-default-features` is checked in CI.
- CI builds the C ABI's example with a C compiler and runs it, and compiles the
  header as C++ as well — `extern "C"` is only load-bearing if somebody does. A
  Windows runner builds `denise-win32` and runs its tests, because it is the only
  machine that can.
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
| **M4** | Text: three font tiers behind feature flags, a bounded glyph atlas, keyboard layouts with dead keys. | ✅ |
| **M5** | C ABI, macOS `NSView`, Windows child-HWND control, ActiveX shim. | ✅ |

M2 does not start until M1 is benchmarked. M5 does not start until the Pi story is
solid — that is the entire point of the project.

M2 shipped legacy modesetting rather than atomic, which reverses the original
plan. What atomic buys is `FB_DAMAGE_CLIPS`, plane composition and tear-free
guarantees; a page flip swaps whole buffers, so damage saves rasterisation rather
than bandwidth, and the one plane worth having — the hardware cursor — has a
legacy equivalent. Atomic slots in behind the same seam when planes earn it.

M3 pulled the built-in bitmap font forward from M4, because a milestone shipping
Label, Button and TextInput without glyphs would have been a milestone in name
only. M4 then found the other half of that gap: `denise-evdev` reported key
*positions* and never turned them into text, so M3's text fields could not receive
a single character from real hardware. Tab and Enter worked, which is why it
looked fine on the Pi.

M4 also added a tier the bootstrap did not name. It listed `cosmic-text` and
`fontdue`; measuring them showed 3.1 MB against 145 KB, and a middle tier with
real fonts but no shaper is what most panels actually want.

M5 was gated on the Pi story being solid, which it was not quite: the console
keyboard was still unmuted, so every character typed into a Denise text field was
also typed at the login shell behind it. That is fixed first — `Console` in
`denise-evdev`, restoring on drop — and then the milestone starts.

M5's ActiveX shim was written twice, and the first attempt was abandoned on
purpose. It would have sat entirely on top of `denise-win32`, which at the time had
never run, and nothing available could have checked it beyond "it compiles" — a
long way from "a container can host it". So the registration table shipped alone,
and the COM object waited until the control underneath it had put a window on a
screen. It then took one sitting, and the container found no bugs in it at all.

What did find one was a test: `2540 / 96` as an integer constant is 26 rather than
26.458, so every extent the control reported was 1.7% short. A container would have
drawn it slightly too small forever and nothing would have pointed at a constant.

Still outstanding, and deliberately not hidden:

- **No type library.** `IDispatch` works, so any late-binding host can set `Text`,
  `Caption` and `Enabled`, call `Refresh` and sink `Change` and `Click`. A `.tlb`
  and a `LIBID` would add a form designer's property sheet, an object browser's
  member list, early binding, and PowerShell without `InvokeMember`.
  `CreateDispTypeInfo` was tried as a substitute and cannot do it — it produces a
  `TKIND_INTERFACE`, and PowerShell wants a dispinterface.
- **No design-time view.** `IViewObject2::Draw` is what a form editor asks for
  before the control is ever activated, so a control dropped on a form is a blank
  rectangle until the form runs.
- **`denise-win32`'s edges are unverified.** It runs, and the input path is
  confirmed on Windows 11 ARM64 — Tab, AltGr, dead keys, hover and mouse-leave.
  Not yet exercised: `SetCapture` on a drag off a pressed button, the wheel's
  screen-to-client conversion, and DPI changes, which is the one I trust least
  because `WM_DPICHANGED` reaches top-level windows only. It has never been
  hosted inside a dialog, which is what `WM_GETDLGCODE` exists for.
  [docs/windows.md](docs/windows.md) is the checklist.
- **Touch is unverified on hardware.** The multitouch slot path is unit tested and
  a single touch is routed to widgets as a pointer would be, but no physical
  touchscreen has driven it.
- **No text selection, clipboard or word motion** in `TextInput`. The measurement
  it needs now exists; the editing model does not.
- **The Norwegian layout is a reconstruction.** `æøå` and the `¨^~` dead key are
  certain; the AltGr assignments on the `+?` and `´` positions are less so, and
  want checking against a physical keyboard.
- **Only two layouts.** US and Norwegian. Adding one is about thirty lines,
  because a layout table lists only what differs from the Latin alphabet.
- **No layout engine.** Nodes are positioned with explicit rectangles relative to
  their parent, which is what a fixed-resolution panel wants; a constraint solver
  can be added over this without changing anything below it.

## Development

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo run -p hello-rect
cargo run -p denise-ui --example showcase -- dark showcase.ppm   # no display needed
cargo run -p denise-text --example specimen -- specimen.ppm      # ditto, for fonts
```

The embedding backends, each on its own platform:

```bash
cargo build -p denise-ffi --release && make -C denise-ffi/examples run
cargo run -p denise-macos --example embed                        # a real window
cargo run -p denise-macos --example embed -- snapshot out.ppm    # no window server
cargo run -p denise-win32 --example embed
```

The macOS snapshot renders through AppKit's own `cacheDisplayInRect:`, so
`drawRect:`, `isFlipped` and the blit all really run — which makes the whole draw
path reviewable over SSH.

The text tiers are off by default, so `--all-features` is the only build that sees
them together and the plain build is the only one that sees neither. CI runs both,
because a `#[cfg]` that compiles in one combination and not the other is exactly
the rot that goes unnoticed.

## Licence

MIT — see [LICENSE](LICENSE).
