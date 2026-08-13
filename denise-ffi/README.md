# denise-ffi

[![crates.io](https://img.shields.io/crates/v/denise-ffi?color=CBA6F7&label=crates.io)](https://crates.io/crates/denise-ffi)
[![docs.rs](https://img.shields.io/docsrs/denise-ffi?color=94E2D5&label=docs.rs)](https://docs.rs/denise-ffi)
[![Licence](https://img.shields.io/badge/licence-MIT-89B4FA)](https://github.com/bisand/denise/blob/main/LICENSE)

A stable **C ABI** for **[Denise]** — a `cdylib` for hosts that are not written in
Rust.

Denise's own backends are Rust and need none of this. This crate exists for the
other direction: a Win32 control inside an MFC application, a WinForms or VB6 host
reaching it through the ActiveX shim, an `NSView` in a Cocoa app, a Python or C#
panel on an embedded box. All of them speak C.

## The shape of it

The host owns the window and the pixel buffer; Denise owns the widget tree and
draws into whatever it is handed.

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

    DeniseRect damage[16];
    intptr_t n = denise_ui_damage(ui, damage, 16);
    /* BitBlt only those rectangles */
    denise_ui_presented(ui);
}

uint32_t message;
while (denise_ui_poll_message(ui, &message)) { /* ... */ }
```

There is no `Surface` here and no event loop. Both belong to the host, and a
library that tried to own either would be unembeddable in exactly the places this
is for.

## Rules the whole ABI keeps

- **Handles are opaque.** A `DeniseUi *` comes from `denise_ui_new` and goes to
  `denise_ui_free`. Nothing else may free it.
- **A node is a `uint64_t`**, and `0` is never valid. Ids carry a generation, so an
  id kept past a remove fails to resolve rather than addressing whoever took the
  slot.
- **A message is a `uint32_t`** chosen by the host, and `0` means *no message* — a
  button given `0` emits nothing, which is what lets a widget exist without one.
- **Strings are NUL-terminated UTF-8** both ways. Invalid UTF-8 is
  `DENISE_ERR_INVALID`, not a replacement character: silently mangling a host's
  text is worse than refusing it.
- **A negative return is a status**, and every status has a message from
  `denise_status_message`.
- **Nothing is thread-safe.** One `DeniseUi` belongs to the thread it was created
  on, which for every host this targets is the UI thread.

## Panics do not cross

Every entry point catches unwinding and returns `DENISE_ERR_PANIC`. A panic is a
bug in Denise, and a bug in Denise should not take down a host process with unsaved
work in three other windows. The call did nothing; the `Ui` should be treated as
suspect and freed.

## The header is the contract

`include/denise.h` is **written by hand, not generated.** A generated header follows
whatever the Rust happens to say this week, which is the opposite of what a stable
ABI means: the header is the thing that must not move, and the Rust is what gets
checked against it. `tests/header.rs` does the checking — every exported symbol
appears in both, with the same numbers for every key, role and constant.

`DENISE_ABI_VERSION` is bumped when a signature, a constant or a meaning changes.
Added functions do not bump it. A host that checks nothing else should check this.

## Building against it

`examples/` has a C program and a `Makefile` that links the `cdylib` and renders to
a PPM — which is also what CI runs, on every push, as *the C ABI links and runs*.

The crate is `crate-type = ["cdylib", "rlib"]`, so a Rust caller can use it directly
too. `unsafe` is necessarily permitted here; every block carries a `// SAFETY:`
comment.

## Where this sits

Wraps [`denise-ui`](https://crates.io/crates/denise-ui).
[`denise-activex`](https://crates.io/crates/denise-activex) is the COM layer that
uses this shape for VB6 and MFC hosts.

## Status

**M5 complete.** Part of [Denise][Denise] — see the [repository README][Denise] for
the whole picture.

MIT licensed.

[Denise]: https://github.com/bisand/denise
