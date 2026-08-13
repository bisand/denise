# denise-ui

[![crates.io](https://img.shields.io/crates/v/denise-ui?color=CBA6F7&label=crates.io)](https://crates.io/crates/denise-ui)
[![docs.rs](https://img.shields.io/docsrs/denise-ui?color=94E2D5&label=docs.rs)](https://docs.rs/denise-ui)
[![Licence](https://img.shields.io/badge/licence-MIT-89B4FA)](https://github.com/bisand/denise/blob/main/LICENSE)

The scene graph, widgets and compositor for **[Denise]**, a direct-rendering UI
toolkit in Rust for embedded Linux and systems without a desktop environment.

A retained tree of widgets in a generational arena, stacked into scenes, drawn
through [`denise-render`](https://crates.io/crates/denise-render) into a
`denise::Surface`. This is the layer that turns "a rasteriser and a display" into
a user interface.

```rust
use denise::{Rect, Role, Size, theme};
use denise_ui::widgets::{Button, Label};
use denise_ui::Ui;

#[derive(Clone, Debug)]
enum Msg { Greet }

# fn demo(surface: &mut impl denise::Surface) -> Result<(), denise::SurfaceError> {
let mut ui: Ui<Msg> = Ui::new(Size::new(800, 480), theme::DARK);
let root = ui.root();
ui.add(root, Label::new("What is your name?"), Rect::new(20, 20, 388, 20));
ui.add(
    root,
    Button::new("Greet", Msg::Greet).with_role(Role::Primary),
    Rect::new(20, 90, 110, 34),
);

loop {
    // ui.handle(&events);
    ui.render(surface)?;            // draws nothing at all when nothing changed
    for message in ui.drain_messages() {
        match message { Msg::Greet => {} }
    }
#   break;
}
# Ok(())
# }
```

## No callbacks

A widget holds a value of *your* message type and emits it when something happens,
so every state change lands in one `match` you wrote — no closures holding
`Rc<RefCell<_>>`, no widget referencing another widget, no `M: Clone` bound.

## The widgets

Twenty of them, deliberately few:

| | |
|---|---|
| `Panel` | A surface with an optional border |
| `Label` | Static text, aligned in its box |
| `Button` | Emits a message of your type |
| `TextInput` | Editing, a caret, and the only widget that animates by default |
| `Checkbox` · `Toggle` | A boolean, as a box or as a switch |
| `RadioGroup` | One choice from a few. **One node, so one tab stop** |
| `Progress` · `Slider` | A value in a range, as output and as input |
| `Divider` · `Badge` · `Alert` | A rule, a pill, a banner |
| `Tabs` · `List` | One selected from many, horizontally or vertically |
| `RadialProgress` | A ring, with room for a number in the middle |
| `Spinner` | An arc that turns. **The one widget that can keep a device awake** |
| `Select` | The closed half of a dropdown; `open_select` is the open half |
| `Image` | A picture with a fit mode. Bring your own pixels — `denise-image` decodes them |
| `Rating` | Stars. Continuous to read, whole stars to set |
| `Avatar` | A picture, or initials on a colour derived from them |

The bar a widget has to clear is being something several panels would otherwise
each get subtly wrong — focus handling, keyboard semantics, hit areas, disabled
states — not saving a caller three `fill_rect` calls. More are being added one at
a time against [issue #6](https://github.com/bisand/denise/issues/6).

Every one names theme *roles* rather than colours, and every surface/foreground
pair is contrast-checked by a test in all three built-in themes.

## Damage is the toolkit's job

There are no dirty flags to set. `Ui::widget_mut` invalidates on access; hover,
press, focus and enabled are tracked by the tree; moving, resizing, showing or
removing a node damages both the old rectangle and the new. `Ui::render` returns
`false` and draws nothing when nothing changed, which is the state a kiosk should
be in almost all the time.

## What is not here

**No layout engine.** Nodes take explicit rectangles relative to their parent,
which is what a fixed-resolution panel wants; a constraint solver can be added over
this without changing anything below it. Widgets with a natural size offer
`preferred_width`/`preferred_height` as *queries the application makes* — the tree
never calls them, and that is the line.

## Scrolling

Mark a node `ui.set_scrollable(view, true)` and it is a viewport: content is
clipped to it, the wheel scrolls it (innermost under the pointer, after the
hovered widget declines), PageUp/Down page the scrollable holding focus, a
touch on its background drags it, and focusing — or moving a `List` selection —
below the fold scrolls the target into view. No smooth scrolling, deliberately:
a kiosk animating a fling at 60 Hz is the idle-cost story in reverse.

## Layers

A modal is `push_scene(dim)`: another root over a dimmed backdrop, with input
and Tab structurally confined to it. A popup is `push_popup(anchor, size, side)`:
anchored to a node, flipped to the other side when the surface runs out, closed
by Escape or a press outside it — which is swallowed, never delivered to what
is underneath — with focus returning to the anchor.

A tooltip is neither, and is not a node either: `ui.set_tooltip(id, "text")`
stores a string, and the tree runs the dwell timer, places the bubble, dismisses
it on any press or key and draws it above every widget. It needs hover, so it
does nothing on a touch-only panel.

`ui.toast(text, role)` is the same arrangement for notifications: the tree
stacks them from the bottom edge, fades them in and out, dismisses one that is
pressed — swallowing the press, so it does not also reach what was underneath —
and removes them without anybody asking. During the hold it costs a single wake,
not a frame rate. `Alert` remains the *inline* banner for the message that has a
place in the layout.

## Features

| Feature | Default | What it does |
|---|:---:|---|
| `std` | ✅ | Off gives `no_std + alloc` |
| `truetype` | | Real TrueType fonts, via `denise-text` |
| `shaping` | | Ligatures, bidi and font fallback, via `denise-text` |

`#![forbid(unsafe_code)]`.

## Status

**M5 complete, M6 in progress.** `cargo run -p denise-ui --example showcase --
dark showcase.ppm` renders every widget in every state to a file, which is how a
theme or font change gets reviewed without a display.

MIT licensed. Part of [Denise][Denise] — see the [repository README][Denise] for
the whole picture.

[Denise]: https://github.com/bisand/denise
