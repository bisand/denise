---
title: Embedding
description: Putting a DeniseUI panel inside an application that already exists — through the C ABI, a macOS NSView, a Win32 child window or an ActiveX control.
---

Everything else in these pages is Denise owning a display. This is the other direction: Denise owning **one rectangle** inside an application that already exists — an MFC dialog, a Cocoa window, a C or C# or Python host.

The shape is the same in all four backends: **the host owns the window, the event loop and the pixel buffer; Denise owns the widget tree and draws into whatever it is handed.** There is no `run` function in any of them, and no `Surface` in the C ABI.

| | Backing store | Present |
|---|---|---|
| `denise-ffi` | the caller's, described by a `DeniseFrame` | the caller's problem |
| `denise-macos` | two `IOSurface`s a `CALayer` reads in place | `setNeedsDisplayInRect:`, then a swap |
| `denise-win32` | a 32-bit top-down DIB section | `InvalidateRect`, then `BitBlt` |
| `denise-activex` | the `denise-win32` control it hosts | the same |

Three things fall out of having done it more than once:

- **Damage means more here.** A page flip swaps whole buffers; `BitBlt` and AppKit move only what they are given. Pass the rectangles on rather than rounding up to the whole control.
- **Row zero is not agreed on.** A `CGImage` is bottom-up, a DIB section is bottom-up unless you ask for a negative height, and Denise's row zero is the top. Neither platform reports a mistake: it renders upside down and looks like the widgets were laid out wrong.
- **There is already a cursor.** Both hosts draw one, so the composited sprite must stay off — `Ui::show_cursor(false)`, a decision that sticks.

## The C ABI

[`denise-ffi`](https://github.com/bisand/denise/tree/main/denise-ffi) is a `cdylib` with a hand-written header. Every host that is not Rust goes through it.

```c
#include <denise.h>

DeniseUi *ui = denise_ui_new(800, 480, DENISE_THEME_DARK);
uint64_t root = denise_ui_root(ui);
denise_ui_add_button(ui, root, (DeniseRect){20, 20, 160, 44},
                     "Save", 1, DENISE_ROLE_PRIMARY);

/* per frame */
denise_ui_tick(ui, now_ms);
if (denise_ui_needs_paint(ui)) {
    DeniseFrame frame = { pixels, len, w, h, stride, DENISE_FORMAT_XRGB8888, age };
    denise_ui_paint(ui, &frame);

    DeniseRect damage[DENISE_MAX_DAMAGE_RECTS];
    intptr_t n = denise_ui_damage(ui, damage, DENISE_MAX_DAMAGE_RECTS);
    /* BitBlt only those rectangles */
    denise_ui_presented(ui);
}

uint32_t message;
while (denise_ui_poll_message(ui, &message)) { /* ... */ }
```

Rules the whole ABI keeps:

- **Handles are opaque.** A `DeniseUi *` comes from `denise_ui_new` and goes to `denise_ui_free`; nothing else may free it, and nothing is thread-safe.
- **A node is a `uint64_t`** and `0` is never valid. Ids carry a generation, so one kept past a remove fails to resolve rather than addressing whoever took the slot.
- **A message is a `uint32_t`** chosen by the host, and `0` means *no message* — a widget given `0` emits nothing.
- **Strings are NUL-terminated UTF-8** both ways, and invalid UTF-8 is refused rather than mangled. A negative return is a status `denise_status_message` describes.
- **Panics do not cross.** Every entry point catches unwinding and returns `DENISE_ERR_PANIC`; the call did nothing, and the `Ui` should be freed.

Keys and text are separate calls, the piece easiest to get wrong from C: `denise_ui_key` carries a position and drives Tab and Enter, while `denise_ui_text` carries what the layout committed, which is the only way `ø` can arrive. On a 2× display, `denise_ui_new_scaled` takes the factor in hundredths.

**The header is the contract.** `include/denise.h` is written by hand and the Rust is checked against it, not generated from it. A test asserts every exported symbol appears in both with the same numbers, because a key number that differs between the two sides is not a link error: the host presses Enter, the field receives Home, and nothing says so. `DENISE_ABI_VERSION` moves when a signature, a constant or a meaning changes.

A complete C host is [`denise-ffi/examples/panel.c`](https://github.com/bisand/denise/blob/main/denise-ffi/examples/panel.c) — with a stride deliberately wider than the width, and a check that Denise never wrote past the visible columns.

```bash
cargo build -p denise-ffi --release && make -C denise-ffi/examples run
```

CI runs exactly that on every push, compiles the header as C++ as well, runs Miri over the crate, and fuzzes the ABI — a target that earned its place on its first run, with an overflow panic reachable from `Ui::tick`.

## macOS: an NSView

[`denise-macos`](https://github.com/bisand/denise/tree/main/denise-macos) is **not a way to ship Denise on a Mac** — `denise-winit` already previews on one. It is for an existing Cocoa application that wants a Denise panel beside its own views.

`DeniseView` is the `NSView` subclass, `ViewSurface` is the `denise::Surface` behind it, and the host implements `ViewDelegate`:

```rust
use denise::{InputEvent, Rect};
use denise_macos::{ViewDelegate, ViewSurface};

impl ViewDelegate for Panel {
    // `damage` arrives empty. Leaving it empty means nothing changed and AppKit
    // is told nothing, which is what makes an idle panel cost nothing.
    fn update(&mut self, surface: &mut ViewSurface, events: &[InputEvent], damage: &mut Vec<Rect>) {
        // handle `events`, paint into `surface`, push what changed onto `damage`
    }

    // A blinking caret is why this exists; the host turns it into an NSTimer.
    fn next_wake_ms(&self) -> Option<u64> {
        None
    }
}
```

The delegate is deliberately not handed a `Ui`: a signage application drawing its own scene has no tree at all.

Pixels live in **two `IOSurface`s**, shown alternately, which CoreAnimation reads in place. The pair is not only about tearing: assigning the *same* object to a layer's `contents` changes no property, so CoreAnimation never looks at the buffer again and the window freezes on its first frame while the application draws into memory nobody reads. Because the buffer is two frames old, `acquire` reports `BufferAge::Frames(2)`. The alternative, a `CGImage` snapshot per frame, measured 9.2% of a core against 0.6%.

```bash
cargo run -p denise-macos --example embed                     # a real window
cargo run -p denise-macos --example embed -- snapshot out.ppm # no window server
```

The snapshot renders through AppKit's own `cacheDisplayInRect:`, so `drawRect:`, `isFlipped` and the blit all really run — the whole draw path, reviewable over SSH.

## Windows: a child HWND

[`denise-win32`](https://github.com/bisand/denise/tree/main/denise-win32) is the oldest reason this project exists: a control that MFC, WinForms and VB6 applications can drop into a dialog next to the ones they already have.

`DeniseControl::new` registers a window class and creates a child `HWND` inside the parent, with a `ControlDelegate` shaped exactly like the Cocoa one — `update(surface, events, damage)` plus `next_wake_ms`, which the host turns into a `SetTimer` interval. Dropping the control is not a destroy: Windows owns the window, which lets a host keep the `HWND` beside its other controls'.

The pixel format already matches, since a 32-bit `BI_RGB` DIB section is what the rasteriser writes. `WM_KEYDOWN` scan codes become key positions and `WM_CHAR` supplies composed text, so AltGr produces `@` and the dead keys produce `é` and `ö` without this crate owning a layout table. A host that does not yet know its scale factor passes `1.0` and corrects it with `DeniseControl::set_scale_factor`.

One note belongs to the `.exe` rather than the library: a Rust binary defaults to the console subsystem, so Windows allocates a console that sits behind the application.

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
```

```bash
cargo run -p denise-win32 --example embed
```

That example is a diagnostic rather than a demo: three lines under the panel report the last key position with its modifiers, the last committed character with its codepoint, and the last pointer position with the frame's damage count. A panel that merely *looks* right tells you nothing about which layer is lying.

**What is verified and what is not.** The control runs on Windows 11 ARM64, where Tab reaches it, AltGr composes and the dead keys work, and a Windows CI runner drives a real control with `SendMessage` on every push — checking that a press captures the mouse, that a drag past the left edge reports a negative x rather than 65533, and that the wheel's screen coordinates are converted. Not proven: **DPI changes**, because `WM_DPICHANGED` reaches top-level windows only. It has never been hosted inside a real dialog, which is what `WM_GETDLGCODE` exists for. [docs/windows.md](https://github.com/bisand/denise/blob/main/docs/windows.md) is the checklist, and carries the toolchain traps too.

## COM and ActiveX

[`denise-activex`](https://github.com/bisand/denise/tree/main/denise-activex) wraps that control for hosts that reach one through the registry: VB6, MFC, Delphi, WinForms. It implements the four `Dll*` exports, a class factory, and a control providing the OLE control interfaces — `IOleObject`, `IOleInPlaceObject`, `IOleWindow`, `IOleControl`, `IPersistStreamInit`, `IDispatch`, `IViewObject2` and `IObjectSafety` — plus the connection point that carries its events.

```text
regsvr32 denise_activex.dll
```

```text
$panel = New-Object -ComObject Denise.Panel
$panel.Caption = "Hei"
$panel.Text
```

The scriptable surface is deliberately short — four members and two events:

| Member | Dispid | |
|---|---|---|
| `Text` | 1 | property, read/write — the field's contents |
| `Caption` | 2 | property, read/write — the heading |
| `Enabled` | 3 | property, read/write |
| `Refresh` | 4 | method — repaint everything |
| `Change` | 1 | event — somebody typed |
| `Click` | -600 | event — the button, at OLE's standard `DISPID_CLICK` |

Hosts that bind names late never needed a type library; PowerShell did, so registration writes `denise_activex.tlb` beside the DLL and registers it. The control can also draw with no site and no window through `IViewObject2::Draw`, which is what a form editor asks for when a control is dropped on a design surface.

It claims to be **safe for scripting**, through `IObjectSafety` and two component categories in the registry, because hosts are split on which they ask. That claim is argued rather than assumed: the surface above is two strings, a boolean and a repaint, and nothing in it opens a file, spawns a process, reads the registry or takes a pointer from the caller. Adding a member that reaches outside the control means making the argument again.

```bash
cargo build -p denise-activex --release
# then, from an elevated prompt:
regsvr32 target\release\denise_activex.dll
cargo run -p denise-activex --example host
```

The example goes through the registry as a real container does, printing each step so each fails distinctly. Its one trap: `cargo run --example host` rebuilds nothing COM will load, because the registry holds a path — so rebuild the release DLL after changing the crate.

**The honest caveat:** no form editor has ever hosted this control. It registers, sites, activates in place, scripts, sinks events and draws its design-time view, all exercised on every push on a Windows runner — but no VB6 form or MFC dialog editor has actually held it.

API reference: [denise-ffi](https://docs.rs/denise-ffi), [denise-macos](https://docs.rs/denise-macos), [denise-win32](https://docs.rs/denise-win32), [denise-activex](https://docs.rs/denise-activex).
