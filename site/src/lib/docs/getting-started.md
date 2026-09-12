---
title: Getting started
description: Add DeniseUI to a project, read the eighty-line hello example, and run it in a window, on a bare display, or straight into a file.
---

DeniseUI is a set of crates rather than one. You pick the core, the widget tree, and **one backend** — the thing that owns the pixels and the input — and that last choice is made at compile time, not discovered at run time.

## Add the crates

The toolkit needs Rust 1.95 or later. All the crates share one version number.

```toml
[dependencies]
denise = "0.23"
denise-ui = "0.23"
denise-winit = "0.23"    # develop on a desktop
# denise-drm = "0.23"    # ship on a display with no compositor
# denise-image = "0.23"  # decode PNG, JPEG, GIF and BMP
```

| Crate | What it is |
|---|---|
| `denise` | Geometry, colour, the pixel buffer contract, input events, damage tracking and theming. `no_std + alloc`, no platform code, `forbid(unsafe_code)`. |
| `denise-ui` | The scene graph, the scene stack, the widgets and the cursor sprite. |
| `denise-winit` | A window on macOS, Windows or Linux, for development and preview. |
| `denise-drm` | Linux DRM/KMS: the display itself, with no X, no Wayland and no compositor. |

**The backend is the application's choice, and it is made with a cargo feature.** A library cannot make it for you: `aarch64-unknown-linux-gnu` is the same target on a kiosk Pi and on a Pi running a desktop, so any probe would be wrong half the time. A feature also means the kiosk build never compiles winit at all.

## The hello example

[`examples/hello`](https://github.com/bisand/denise/blob/main/examples/hello/src/main.rs) is the smallest useful Denise application: type a name, press a button, read a greeting. It is about eighty lines, roughly half of them comments, and it has three pieces.

### 1. A message type

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
enum Message {
    Greet,
}
```

Widgets do not run callbacks. A button holds a value of *your* type and emits it when pressed, so every state change in the application happens in one `match` you wrote, and the compiler tells you when you forget a variant.

### 2. A tree

`Ui::add` places a widget under a parent, at a rectangle **relative to that parent**, and hands back a `NodeId` — which is how you reach the widget again later.

```rust
let mut ui: Ui<Message> = Ui::new(size, theme::DARK.scaled(scale));
let root = ui.root();

let card = ui
    .add(root, Panel::default(), Rect::new(16, 16, 428, 228))
    .expect("card");

ui.add(card, Label::new("What is your name?").with_size(16), Rect::new(20, 58, 388, 20));

let name = ui
    .add(
        card,
        TextInput::<Message>::new()
            .with_placeholder("your name")
            .with_submit(Message::Greet),
        Rect::new(20, 82, 388, 34),
    )
    .expect("field");

ui.add(
    card,
    Button::new("Greet", Message::Greet).with_role(Role::Primary),
    Rect::new(20, 128, 110, 34),
);

ui.focus(Some(name));
```

A few things to notice, all of which hold across the toolkit:

- **There is no layout engine.** Every node is an explicit rectangle relative to its parent, which is what a fixed-resolution panel wants. The real example centres its card with arithmetic against the surface size, so that card rectangle is the only one in the file that knows how big the screen is. Anchors, docking and a vertical stack exist as placement rules; content-driven sizing belongs to an optional crate, `denise-arrange` ([design note](https://github.com/bisand/denise/blob/main/docs/arrange.md)).
- **Widgets name roles, not colours.** `Role::Primary` asks the theme for a surface and a foreground guaranteed to contrast with it. Swapping `theme::DARK` for `theme::LIGHT` is the whole of supporting both — see [Theming](../theming/).
- **`with_submit` makes Enter emit the same message as the button**, which is what a keypad-only panel needs.

### 3. Handling what comes back

The only piece of application logic reads the field and writes the label:

```rust
fn greet(&mut self) {
    let name = self
        .ui
        .widget::<TextInput<Message>>(self.name)
        .map(|field| field.text().trim().to_string())
        .unwrap_or_default();

    if let Some(label) = self.ui.widget_mut::<Label>(self.greeting) {
        label.set_text(format!("Hello, {name}."));
    }
}
```

`widget_mut` marks the node for repaint **on access**. Taking a mutable reference to a widget is the declaration that it will look different, so there is no `invalidate()` to forget.

### The loop

With `denise-winit`, the application implements `DeniseApp`. Once per pass it hands the tree its events, advances the clock, drains messages, and reports what changed:

```rust
self.ui.handle(events);
self.ui.tick(self.started.elapsed().as_millis() as u64);

// Collected first: draining borrows the tree, and handling a message needs it back.
let messages: Vec<Message> = self.ui.drain_messages().collect();
for message in messages {
    match message {
        Message::Greet => self.greet(),
    }
}

// What changed, not the window.
if self.ui.needs_paint() {
    let pending = self.ui.pending_damage();
    if pending.is_empty() {
        damage.add_full();
    } else {
        for rect in pending {
            damage.add(*rect);
        }
    }
}
```

Painting is `ui.paint(frame)` followed by `ui.presented()`. The example also answers `next_frame_in` from `Ui::next_wake_ms`: the caret wants waking every half second, and saying so is the difference between sixty frames a second and two. A tree with nothing animating asks to be woken never.

There are no dirty flags anywhere in that. Type into the field and the toolkit repaints the field, not the window.

## Run it

```bash
cargo run -p hello                                          # a window
cargo run -p hello --no-default-features --features kiosk   # the display itself
```

The second line is Linux only. It swaps the window for the display and evdev input, mutes the console keyboard for the life of the process, and never links winit. The code that differs lives in its own `kiosk.rs`, so `main.rs` stays about the tree. On the kiosk build Escape quits and F12 writes a screenshot to `/tmp/denise-hello.ppm`.

To build it for a Raspberry Pi from another machine:

```bash
cargo build -p hello --no-default-features --features kiosk \
    --release --target aarch64-unknown-linux-musl
```

Read [Raspberry Pi](../raspberry-pi/) before running that on a board — a stock Pi has no `/dev/dri` until the vc4 KMS overlay is enabled.

## Draw a frame into a file

Several examples take `--snapshot`, which draws one frame into a PPM and exits. It needs no display, which makes it the way to review a layout over SSH or diff a theme change before and after.

```bash
cargo run -p hello -- --snapshot hello.ppm
cargo run -p hello -- --snapshot out.ppm 2        # the same layout at a 2x scale factor
cargo run -p denise-ui --example showcase -- dark showcase.ppm
```

The snapshot path works because a `Frame` is only a borrowed pixel buffer with a size, a stride, a format and an age. Anything that can lend one — a window, a scanout buffer, a `Vec` — can be drawn into.

## Text, in one paragraph

`hello` draws in the built-in 8×8 bitmap font because it never asks for another. Real faces are the `truetype` feature (about 145 KB) and full shaping is `shaping` (about 3.1 MB), both passed through `denise-ui`. [How it works](../how-it-works/) has the trade-off.

## What is deliberately missing

From the README's known gaps: no layout engine, no undo in `TextInput` (selecting, word motion and the clipboard are there), three keyboard layouts (US, Norwegian, German), and touch input that is unit tested but has not been driven by a physical touchscreen.

## Where next

- [`examples/table-editor`](https://github.com/bisand/denise/tree/main/examples/table-editor) — the same idea grown up: a scrolling grid, an edit form, validation, a confirmation modal and CSV persistence.
- [`examples/gallery`](https://github.com/bisand/denise/tree/main/examples/gallery) — every widget live, with a theme editor beside them: `cargo run -p gallery`.
- [How it works](../how-it-works/) — damage, scenes, scrolling, motion and what an idle panel costs.
- [Theming](../theming/) — roles, seeds and the DPI story.
- [Forms and the designer](../forms-and-designer/) — the same tree from a `.dform` file instead of Rust.
- [Embedding](../embedding/) — a Denise panel inside a C, Cocoa or Win32 application.
- API documentation: [docs.rs/denise](https://docs.rs/denise) and [docs.rs/denise-ui](https://docs.rs/denise-ui).
