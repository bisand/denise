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

## On a HiDPI display

`WindowConfig::size` is **logical**, so one number is the same amount of desk
everywhere: 1280×800 on a Raspberry Pi is a 1280×800 surface, and on a 2× Retina
Mac it is a window of the same apparent size with a 2560×1600 surface behind it.

Filling that surface is the application's job, and `run_with` is how it finds
out it has to:

```rust
# use denise::{DamageTracker, Frame, InputEvent, Rect, Size};
# use denise_winit::{DeniseApp, WindowConfig, run_with};
# struct Panel;
# impl Panel { fn new(_: Size, _: f32) -> Self { Panel } }
# impl DeniseApp for Panel {
#     fn update(&mut self, _: &[InputEvent], _: &mut DamageTracker) {}
#     fn render(&mut self, _: &mut Frame<'_>, _: &[Rect]) {}
# }
# fn demo() {
// The surface, in physical pixels, and the display's scale factor — handed over
// at the first moment either exists. The application scales once, here, through
// `Theme::scaled`, `Rect::scaled` and its own text sizes.
run_with(WindowConfig::default(), Panel::new).unwrap();
# }
```

A later scale change — dragging the window to a second display — arrives as
`InputEvent::SurfaceResized`, carrying the new factor.

## Closing

The window manager's close button ends the run, and the request also arrives as
`InputEvent::CloseRequested` so an application can save on the way out. An
application that needs to *stop* the close — unsaved changes, a confirmation —
overrides `DeniseApp::close_requested` to return `false`, and quits later
through `exit_requested` once it has its answer.

## What a frame costs here, and why it is not what a panel costs

A frame in which nothing changed costs nothing: the loop skips the acquire, the
paint and the present entirely. An idle window sits at well under 1% of a core.

A frame in which *something* changed is a different matter on this backend, and
the number does not transfer to the hardware. On macOS, softbuffer's
CoreGraphics backend pays the whole surface three times per present, whatever
the damage says:

- `buffer_mut` allocates and zeroes a **fresh** buffer every call, so the buffer
  it hands back is always age-undefined and the shadow has to be copied into it
  in full;
- `present_with_damage` discards its damage argument and calls `present`;
- CoreAnimation then copies the resulting `CGImage` into a texture —
  `CGContextDrawImage`, again the whole surface.

On a 2560×1600 Retina surface that is roughly 48 MB of memory traffic per
present. `examples/hello-rect` prints the contradiction as it runs: *1.0% of the
surface repainted*, beside a CPU figure that says nothing of the sort.

So the only lever here is **presents per second**, which is why
[`DeniseApp::next_frame_in`] exists and why an application should answer it from
`Ui::next_wake_ms` rather than let the loop free-run. A `Spinner` asks for 50 ms;
honouring that instead of ticking it at 60 Hz is a threefold difference in this
backend's CPU, and no difference at all in what the user sees.

[`DeniseApp::next_frame_in`]: https://docs.rs/denise-winit/latest/denise_winit/trait.DeniseApp.html#method.next_frame_in

So a widget that animates continuously — a spinner, a carousel mid-slide — is
expensive **here** and cheap on the target, where `present` is a page flip and
the only cost is rasterising the damage. Judge idle cost on this backend; judge
animation cost on the panel.

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
