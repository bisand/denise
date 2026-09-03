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

## Secondary windows

A desktop has a window manager, so a settings form can be a *window* rather than a
scene in the same buffer. `DeniseApp::take_windows` is the whole of it: hand back a
`WindowRequest` and the backend opens a window with its own surface, its own damage
tracker and its own frame deadline, running an application you built.

```rust
# use denise::{DamageTracker, Frame, InputEvent, Rect, Size};
# use denise_winit::{DeniseApp, Modality, WindowConfig, WindowRequest};
# struct Settings;
# impl Settings { fn new(_: Size, _: f32) -> Self { Settings } }
# impl DeniseApp for Settings {
#     fn update(&mut self, _: &[InputEvent], _: &mut DamageTracker) {}
#     fn render(&mut self, _: &mut Frame<'_>, _: &[Rect]) {}
# }
# struct Main { wanted: bool }
# impl Main {
fn take_windows(&mut self) -> Vec<WindowRequest> {
    if !std::mem::take(&mut self.wanted) {
        return Vec::new();
    }
    // Modeless and owned by the window that asked, which is the default: above it,
    // closed with it, and the main window stays usable. `Modality::Modal` blocks
    // the owner instead; `Modality::Independent` is a window of its own.
    vec![WindowRequest::new(WindowConfig::default(), Settings::new)]
}
# }
```

A form is an ordinary `DeniseApp` — the same trait the main window implements — so
there is no form type, no base class, and nothing that makes a "dialog" different
from a "window" except the `Modality` asked for. It is built through the same
`(Size, f32)` callback `run_with` uses, because a form opens on whichever display
its owner is on and needs that display's scale factor.

Closing the **main** window ends the run. Closing any other window closes that
window and everything it opened. Nothing can close a window it did not build: a
form ends itself through `exit_requested`, which is also how the window that opened
it asks — through state they share, which the application owns and this crate never
sees.

**Modality is enforced here, not by the platform.** A window with a modal over it
stops receiving input and keeps repainting; a press on it raises the modal instead.
That is the same on all three platforms, which the platforms themselves are not:

| | Owned z-order | Owner blocked |
|---|---|---|
| Windows | `with_owner_window` | `set_enable(false)` — a real Win32 modal |
| macOS | `addChildWindow:ordered:` | nothing; `runModal` would fight winit's loop |
| X11 / Wayland | nothing reachable through winit | nothing |

So the platform calls are appearance, and deleting them would cost looks rather
than correctness. On Linux the window manager may put a modal behind its owner;
it still cannot be typed into. `cargo run -p forms` is the whole feature in one
example.

**This is desktop-only and stays here.** `denise-ui` is untouched by it and knows
nothing about a second tree; a kiosk build links `denise-drm` and never compiles a
line of this. The portable way to ask a question is still `Ui::push_scene`.

## What a frame costs here, and why it is not what a panel costs

A frame in which nothing changed costs nothing: the loop skips the acquire, the
paint and the present entirely. An idle window sits at well under 1% of a core.

A frame in which something changed is presented by the platform's own path, and
those differ more than they should. win32 `BitBlt`s the damage rectangles into a
persistent DIB section; x11, wayland and kms do the equivalent; all of them
report a real buffer age, so only what changed is ever copied.

**macOS does not go through softbuffer at all here.** Its CoreGraphics backend
allocates and zeroes a fresh buffer on every `buffer_mut`, reports an age of 0 so
the shadow has to be copied in full, and discards the damage rectangles in
`present_with_damage` — three passes over the whole surface per frame, which on
a 2560×1600 Retina window cost 48.8% of a core to animate one spinner. So this
backend presents through [`denise-macos`] instead: a pair of `IOSurface`s the
compositor reads in place, alternated so CoreAnimation sees a new object each
frame. The same window, the same spinner, at 60 frames a second rather than 20:

| | CPU |
|---|---|
| softbuffer, 60 fps | 48.8% |
| softbuffer, 20 fps | 24.5% |
| `IOSurface` pair, 60 fps | **3.5%** |

For reference, the same tree on a Raspberry Pi 3A+ over DRM is 4.2%, and on
Windows 0.5%. The desktop backends are now within sight of the hardware, which
is the only claim this crate ever wanted to make.

How much of that a panel spends is the application's decision rather than this
crate's: `Ui::set_motion` sets the rate everything animates at, and
`Motion::None` leaves the tree asking for no wake at all — at which point
`next_frame_in` answers `None` and the loop blocks on input, the state a kiosk
should be in almost all the time. `cargo run -p gallery -- --motion 33` is the
lever in one flag.

## Why it earns its place

Because the alternative is developing blind. A backend that produces the *same*
`InputEvent`s and honours the *same* `Surface` contract as DRM means a panel can be
written and reviewed on a laptop and then run unchanged on the hardware — and when
it does not, the difference is in the backend rather than in the abstraction, which
is a far smaller place to look.

It also keeps the contract honest: two independent implementations of `Surface` is
the minimum at which "the trait describes the problem" stops being an assertion.

Runs on Linux, macOS and Windows. Windowing and input come from **winit**
everywhere; presentation comes from **softbuffer**, except on macOS where it
comes from [`denise-macos`] for the reasons above — or, behind the `gpu`
feature, from `denise-wgpu` on any of them. They are the only
dependencies in the whole workspace that are there purely for convenience.

## On the GPU

Behind the `gpu` feature, a window can present through
[`denise-wgpu`](https://crates.io/crates/denise-wgpu) instead of a buffer of
words. Nothing about the window, the input or the scheduling changes; only what
draws. Ask for it with `Present::Gpu`, and draw through `paint` instead of
`render`:

```rust,no_run
use denise::{BufferAge, DamageTracker, InputEvent, Pen, Rect, theme};
use denise_ui::Ui;
use denise_winit::{DeniseApp, Present, WindowConfig, run_with};

struct Panel {
    ui: Ui<()>,
}

impl DeniseApp for Panel {
    fn update(&mut self, events: &[InputEvent], _damage: &mut DamageTracker) {
        self.ui.handle(events);
    }

    // The painter-agnostic half. On the GPU `age` is always `Undefined`; on
    // the software path `render` is provided and calls this with the frame's.
    fn paint(&mut self, pen: &mut Pen<'_>, age: BufferAge, _damage: &[Rect]) -> bool {
        self.ui.paint_with(pen, age);
        self.ui.presented();
        true
    }
}

# fn main() -> Result<(), denise_winit::Error> {
run_with(
    WindowConfig {
        title: "On the GPU".into(),
        present: Present::Gpu,
        ..WindowConfig::default()
    },
    |size, scale| Panel {
        ui: Ui::new(size, theme::DARK.scaled(scale)),
    },
)
# }
```

Every GPU frame is a full repaint. A swapchain keeps no reliable buffer age and
a desktop GPU redraws a window for nothing, so the damage tracker's work is not
needed there and `BufferAge::Undefined` is what tells the application so. An
application that implements only `render` — one that wants a `Frame` — gets
`Error::Gpu` at window creation rather than a blank window.

This is for the designer on a large display. A preview of a panel does not
need it, and a panel never has it: the kiosk path is unchanged.

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
[`denise-macos`]: https://crates.io/crates/denise-macos
