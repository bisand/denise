# denise-winit

[![crates.io](https://img.shields.io/crates/v/denise-winit?color=CBA6F7&label=crates.io)](https://crates.io/crates/denise-winit)
[![docs.rs](https://img.shields.io/docsrs/denise-winit?color=94E2D5&label=docs.rs)](https://docs.rs/denise-winit)
[![Licence](https://img.shields.io/badge/licence-MIT-89B4FA)](https://github.com/bisand/denise/blob/main/LICENSE)

The desktop development and preview backend for **[Denise]**, a direct-rendering UI
toolkit in Rust for embedded Linux and systems without a desktop environment.

**This is not a deployment target.** It exists so the core abstraction can be
proven — and iterated on — without a Raspberry Pi on the desk. Shipping Denise on a
desktop means shipping a compositor you did not need.

```rust
use denise::{Color, DamageTracker, Frame, InputEvent, Rect};
use denise_render::Canvas;
use denise_winit::{DeniseApp, WindowConfig, run};

struct Hello;

impl DeniseApp for Hello {
    fn update(&mut self, _events: &[InputEvent], _damage: &mut DamageTracker) {}

    fn render(&mut self, frame: &mut Frame<'_>, damage: &[Rect]) {
        let mut canvas = Canvas::new(frame);
        for region in damage {
            canvas.with_clip(*region).clear(Color::from_rgb888(0x1E1E2E));
        }
    }
}

# fn demo() {
run(WindowConfig::default(), Hello).unwrap();
# }
```

## Why it earns its place

Because the alternative is developing blind. A backend that produces the *same*
`InputEvent`s and honours the *same* `Surface` contract as DRM means a panel can be
written and reviewed on a laptop and then run unchanged on the hardware — and when
it does not, the difference is in the backend rather than in the abstraction, which
is a far smaller place to look.

It also keeps the contract honest: two independent implementations of `Surface` is
the minimum at which "the trait describes the problem" stops being an assertion.

Runs on Linux, macOS and Windows, on **winit** and **softbuffer** — the only two
dependencies in the whole workspace that are there purely for convenience.

## Where this sits

Implements `denise::Surface` and `denise::InputSource`. Swap it for
[`denise-drm`](https://crates.io/crates/denise-drm) plus
[`denise-evdev`](https://crates.io/crates/denise-evdev) to ship, usually behind one
`cfg` in the application.

For putting a Denise panel *inside* an existing desktop application, this is the
wrong crate — see [`denise-win32`](https://crates.io/crates/denise-win32),
[`denise-macos`](https://crates.io/crates/denise-macos) or
[`denise-activex`](https://crates.io/crates/denise-activex), which embed rather
than own the window.

## Status

**M0 complete**, and used continuously since. Part of [Denise][Denise] — see the
[repository README][Denise] for the whole picture.

MIT licensed.

[Denise]: https://github.com/bisand/denise
