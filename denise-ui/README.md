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

Twenty-five of them, deliberately few:

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
| `Table` | Cells under a pinned header. **Windows its data, so row count is free** |
| `Timeline` | Events in order: time, disc, connector, label |
| `Carousel` | Pictures sliding on the advance clock. **One wake per hold** |
| `Collapse` | A section that folds to its header; `Accordion` adds exclusivity |
| `Video` | The rectangle a video plane sits in — the frames never come through the tree |

The bar a widget has to clear is being something several panels would otherwise
each get subtly wrong — focus handling, keyboard semantics, hit areas, disabled
states — not saving a caller three `fill_rect` calls. More are being added one at
a time against [issue #6](https://github.com/bisand/denise/issues/6).

Every one names theme *roles* rather than colours, and every surface/foreground
pair is contrast-checked by a test in all three built-in themes.

## Widgets describe their own properties

Every widget implements `Describe`: what a form file calls it, what settings it
has, and how to read and write each one by name.

```rust
use denise_ui::widgets::{Button, Describe, Value};
use denise_ui::Void;

let mut button = Button::<Void>::inert("Save");
button.set("text", Value::text("Apply")).unwrap();
assert_eq!(button.get("text"), Some(Value::text("Apply")));
```

The list lives beside the widget so that nothing else has to keep a copy. Two
things want one — [`denise-forms`](../docs/forms.md), which builds a tree from a
`.dform` file, and the designer's property inspector, which shows one editor per
property — and a table maintained in either would drift from the widgets the
first time one grew a setting. `widgets::all()` is the catalogue a palette lists,
and a test reads `mod.rs` and fails if a widget is missing from it.

A widget also declares **what it is** — one line, `Describe::DOC` — and **which
shelf it stands on**, one of six `Group`s. That is what lets a designer's palette
group twenty-seven rows and say what each is without keeping a table of
descriptions, which is the thing this whole module exists to avoid. Both are
required rather than defaulted: a widget nobody can identify in a palette is a
widget nobody reaches for, so adding one without a line does not compile.

Through the tree it is `ui.set_property(id, "role", Value::Enum("primary"))`,
which is the single place a string becomes a typed call on a widget. An error
names the widget, the property, and everything that *would* have been accepted.

Two kinds of property are described but not settable here, and
`Property::is_settable` says which. A **message** is a value of your type, and
this crate has never seen your type. An **asset** is a path, and this crate
decodes nothing. The engine resolves both and passes them to the constructor.

The ranges on a numeric property are what an editor should offer, not a gate: a
widget that clamps clamps a value from `set` exactly as it clamps one from its
own setter, so there is one rule rather than two.

This is what lets the [visual designer](https://github.com/bisand/denise/tree/main/tools/designer)
contain **no list of widgets anywhere**. Its palette is `widgets::all()`, its
inspector draws one editor per `Property` and reads the tooltip from that
property's own documentation, and a widget added to this crate turns up in both
without anybody editing the designer. The properties the *tree* owns — geometry,
visibility, docking — and the ones the *form node* owns are described the same
way, in `denise-forms`, so the inspector has one rule for all three.

## Anchors and docking

A node keeps its rectangle when its parent resizes — unless it says otherwise.
The two ways to say otherwise are the two Delphi and the WinForms designer had.

```rust
# use denise::{Rect, Size, theme};
# use denise_ui::widgets::Panel;
# use denise_ui::{Ui, Void};
use denise_ui::{Anchors, Dock};
# let mut ui: Ui<Void> = Ui::new(Size::new(320, 200), theme::DARK);
# let root = ui.root();
# let toolbar = ui.add(root, Panel::default(), Rect::new(0, 0, 0, 32)).unwrap();
# let field = ui.add(root, Panel::default(), Rect::new(8, 40, 304, 24)).unwrap();
# let ok = ui.add(root, Panel::default(), Rect::new(252, 168, 60, 24)).unwrap();

ui.set_dock(toolbar, Some(Dock::Top));                          // full width, at the top
ui.set_anchors(field, Anchors::new(true, true, true, false));   // stretches
ui.set_anchors(ok, Anchors::new(false, false, true, true));     // stays bottom-right
# assert_eq!(ui.bounds(toolbar).unwrap(), Rect::new(0, 0, 320, 32));
```

`Anchors` names the parent edges a node keeps its distance from: one edge of an
axis and it holds its place and its size, both and it **stretches**, neither and
its two gaps grow equally so a centred node stays centred. The default is
`TOP_LEFT`, which derives exactly the rectangle the node already had — so a tree
that never mentions anchoring behaves as it always did.

`Dock` gives a node an edge of what is *left* of its parent, in paint order, so
two bars docked to the top are two stacked bars and a `Dock::Fill` takes the rest.
Docking shrinks the box the undocked children are placed in, which is why docking
a toolbar moves the form down instead of covering it.

Neither is a solver, and neither rewrites a node's `layout`: both **derive** a
rectangle in `reflow`, the one pass that turns layouts into bounds — the same
place the vertical stack and the scroll offset happen, and for the same reason.
Paint, damage, clipping and hit testing all read what that pass wrote, so they
cannot disagree about where a node ended up. It is also what keeps a form file to
one rectangle per node: a designer moving a button still produces a one-line diff,
whatever the anchors do with it afterwards.

## Damage is the toolkit's job

There are no dirty flags to set. `Ui::widget_mut` invalidates on access; hover,
press, focus and enabled are tracked by the tree; moving, resizing, showing or
removing a node damages both the old rectangle and the new. `Ui::render` returns
`false` and draws nothing when nothing changed, which is the state a kiosk should
be in almost all the time.

## One knob for how fast animation runs

```rust,ignore
ui.set_motion(Motion::Every(33));  // 30 fps: half the wakes, half the cost
ui.set_motion(Motion::None);       // reduced motion, or a tight power budget
```

Every moving thing in the tree runs at this rate — spinners, knobs crossing,
carousel slides, layout tweens, toast fades — because a widget says *that* it is
moving (`Wake::Animating`) and the tree says *when*. It used to be four private
constants in four widgets, which is one decision copied four times and reachable
from nowhere.

It is a **sample rate and not a duration**. A toggle still crosses in 120 ms and
a carousel still advances after eight seconds at any setting: those are
deadlines, spelled `Wake::At`, and turning the rate down draws a transition in
fewer positions rather than making it take longer. Quantising a schedule to a
frame rate would be a bug, so the two are different words.

`Motion::None` is not merely a very slow rate. Transitions land at their end
state at once, the animating set empties, and the tree asks for no wake at all —
the `prefers-reduced-motion` answer, and the right setting where any animation
is a bad trade. Schedules survive it: a tooltip still appears after its dwell, a
toast still goes after its hold, a carousel still rotates. It cuts between
pictures instead of sliding between them.

The default is 16 ms, because sixty is what stops a turning arc reading as a
stutter. What that costs is the gallery on a Pi 3A+ over DRM, one spinner
turning, with nothing changed between runs but the flag:

| | CPU |
|---|---|
| 16 ms, the default | 4.20% |
| 33 ms | 2.06% |
| 50 ms | 1.26% |
| off | **0.00%** |

Which of those is right depends on the deployment, which is why it is a setting
on the tree and not a constant in a widget — and not on `Theme`, since swapping
dark for light should not change the power budget. A single widget that
genuinely differs can still override it, `Spinner::with_frame_ms` being the one
that does.

## What is not here

**No layout engine.** Nodes take explicit rectangles relative to their parent,
which is what a fixed-resolution panel wants. Widgets with a natural size answer
`Widget::measure` — and `preferred_width`/`preferred_height` where they had one
first — as *queries the application makes*. **The tree never asks**, and that is
the line: an intrinsic-size protocol is one where the tree asks every widget how
big it wants to be and then places it, and nothing in this crate consumes any of
this.

`ui.measure(id, Offer::NOTHING)` is how a caller who holds a `NodeId` rather than
the widget asks. It exists because of a borrow — measuring needs the widget and
the text engine at once, and both live inside `Ui` — and because the inherent
queries never agreed on a signature. `Offer` and `Measured` are per axis, because
the widgets are: an `Alert` has a height for a width you promise it and no view
about its own width; a `Panel` has no view about either, and says so rather than
inventing one.

There are two *placement rules* over those rectangles — see [Anchors and
docking](#anchors-and-docking) — and neither is a solver: each derives one
rectangle per child in a pass that already runs. Content-driven sizing, where a
label is as wide as its text, is the thing this does not do, and it lives in
[`denise-arrange`](https://github.com/bisand/denise/tree/main/denise-arrange), a
crate an application opts into rather than something in here.

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
does nothing on a touch-only panel. The one thing the tree cannot decide for
itself is how big the text is, because it does not know the display's scale
factor: `ui.set_tooltip_size(px)` is the lever, and a scale-aware application
pulls it once alongside its other sizes.

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
