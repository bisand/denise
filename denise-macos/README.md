# denise-macos

[![crates.io](https://img.shields.io/crates/v/denise-macos?color=CBA6F7&label=crates.io)](https://crates.io/crates/denise-macos)
[![docs.rs](https://img.shields.io/docsrs/denise-macos?color=94E2D5&label=docs.rs)](https://docs.rs/denise-macos)
[![Licence](https://img.shields.io/badge/licence-MIT-89B4FA)](https://github.com/bisand/denise/blob/main/LICENSE)

An embeddable Cocoa view backend for **[Denise]**, a direct-rendering UI toolkit in
Rust for embedded Linux and systems without a desktop environment.

**Not a way to ship Denise on a Mac.**
[`denise-winit`](https://crates.io/crates/denise-winit) already previews on one, and
a desktop application should use a desktop toolkit. This exists for the same reason
the Win32 control does: an existing Cocoa application that wants a Denise panel
*inside* it, next to its own views, with the host owning the window and the run
loop.

```rust
# #[cfg(target_os = "macos")]
# fn demo() -> Result<(), denise_macos::Error> {
use denise::Size;
use denise_macos::ViewSurface;

// The host has a view; Denise has a surface the size of its backing store.
let mut surface = ViewSurface::new(Size::new(800, 480), 2.0)?;
# let _ = &mut surface;
# Ok(())
# }
```

`DeniseView` is the `NSView` subclass, `ViewDelegate` is what the host implements to
drive it, and `ViewSurface` is the `denise::Surface` behind a layer-backed context.

## What is different from the bare-metal backends

- **The host owns the run loop.** There is no `run` function here. AppKit decides
  when to draw and Denise answers — the opposite of the DRM backend, where Denise
  decides and the display follows.
- **Damage is real.** `setNeedsDisplayInRect:` genuinely limits what gets
  composited, unlike a page flip where the whole buffer goes regardless. So the
  rectangles the tree produces are worth passing on rather than rounding up to the
  whole view.
- **Points are not pixels.** A Retina view is 2 physical pixels per point. Denise
  lays out in physical pixels throughout — the conversion happens once, at this
  edge, and nothing above it needs the scale factor to hit-test.
- **There is already a cursor.** The host's window system draws one, so the
  composited sprite stays off: `Ui::show_cursor(false)`, a decision that sticks
  rather than one the next mouse move overrides.

## Platform

macOS only; elsewhere the crate compiles to almost nothing. Built on **objc2**, so
the Objective-C bridging is checked rather than hand-rolled. `unsafe` is necessarily
permitted here; every block carries a `// SAFETY:` comment.

`examples/embed.rs` is a complete host — a window, a view and the run loop:

```text
cargo run -p denise-macos --example embed
```

## Where this sits

Wraps [`denise-ui`](https://crates.io/crates/denise-ui). The Windows equivalents are
[`denise-win32`](https://crates.io/crates/denise-win32) and
[`denise-activex`](https://crates.io/crates/denise-activex).

## Status

**M5 complete**, run on real hardware. Part of [Denise][Denise] — see the
[repository README][Denise] for the whole picture.

MIT licensed.

[Denise]: https://github.com/bisand/denise
