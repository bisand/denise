# denise-win32

[![crates.io](https://img.shields.io/crates/v/denise-win32?color=CBA6F7&label=crates.io)](https://crates.io/crates/denise-win32)
[![docs.rs](https://img.shields.io/docsrs/denise-win32?color=94E2D5&label=docs.rs)](https://docs.rs/denise-win32)
[![Licence](https://img.shields.io/badge/licence-MIT-89B4FA)](https://github.com/bisand/denise/blob/main/LICENSE)

A **[Denise]** panel in a Win32 child window.

The oldest of the reasons this project exists. Its predecessor shipped inside
Windows applications that were not going to be rewritten — MFC, WinForms, VB6
through the ActiveX shim — and the thing they all need is a control they can put in
a dialog next to the ones they already have.

So `DeniseControl` registers a window class and creates a child `HWND`. **The host
owns the window, the message loop and the parent; Denise owns the pixels inside one
rectangle and nothing else.**

## What is different from the bare-metal backends

- **The host owns the message loop.** There is no `run` function here. Windows
  decides when to paint and Denise answers — the opposite of the DRM backend, where
  Denise decides and the display follows.
- **Damage is real bandwidth.** `BitBlt` moves only the rectangles it is given,
  unlike a page flip where the whole buffer goes regardless. The tree's damage is
  worth passing on rather than rounding up to the client area.
- **The pixel format already matches.** A 32-bit `BI_RGB` DIB section is
  `0xAARRGGBB` in a little-endian `DWORD`, exactly what the rasteriser writes. No
  conversion pass anywhere.
- **There is already a cursor.** Windows draws one, so the composited sprite stays
  off — `Ui::show_cursor(false)`, which is a decision that sticks rather than one
  the next mouse move overrides.

## No console window

A Rust binary defaults to the **console** subsystem, so Windows allocates a console
that sits behind the app looking like a mistake. The `.exe` hosting the control —
not this library — decides that:

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
```

Keeping the console while developing and dropping it for the build that ships,
because the cost of `windows_subsystem = "windows"` is that `stdout`, `stderr` and
panic messages all go nowhere.

## Keyboard

`WM_KEYDOWN` scan codes are translated to Denise's `KeyCode`, and `WM_CHAR` supplies
composed text — so AltGr produces `@` and the dead keys produce `é` and `ö` without
this crate owning a layout table. Tab reaches the control and moves focus inside it
like any other Windows control.

## Platform

Windows only; elsewhere the crate compiles to almost nothing, which is what lets the
whole workspace be checked and published from one runner. Built and tested on
**Windows 11 ARM64** in CI, on every push, as *the Win32 control builds and its
tests run*. `unsafe` is necessarily permitted here; every block carries a
`// SAFETY:` comment.

## Where this sits

Wraps [`denise-ui`](https://crates.io/crates/denise-ui) and provides the window that
[`denise-activex`](https://crates.io/crates/denise-activex) hosts for COM
containers. `examples/embed.rs` is a complete host: a parent window, a message loop,
and the control in it.

## Status

**M5 complete**, run on real hardware — Windows 11 on ARM64, where Tab reaches the
control, AltGr composes and the dead keys work. Part of [Denise][Denise] — see the
[repository README][Denise] for the whole picture.

MIT licensed.

[Denise]: https://github.com/bisand/denise
