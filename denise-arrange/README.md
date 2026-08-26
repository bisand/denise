# denise-arrange

Content-driven layout for [DeniseUI](https://github.com/bisand/denise), in a crate
you depend on or do not.

```toml
[dependencies]
denise-arrange = "0.19"
```

`denise-ui` has **no layout engine**, and that is a feature: nodes take explicit
rectangles relative to their parent, which is what a fixed-resolution panel wants.
This is the layer *over* that for applications which want a label as wide as its
text, a row that grows with what is in it, or a toolbar that distributes what is
left. It computes rectangles and calls `Ui::set_layout` — exactly what an
application doing its own arithmetic would call, which is why the tree needs no
knowledge of it and why linking none of this costs nothing.

The full argument, including what was given up and why, is in
[docs/arrange.md](https://github.com/bisand/denise/blob/main/docs/arrange.md).

## Three ways a child is sized

```rust
use denise::{Rect, Size, theme};
use denise_ui::{Ui, Void, widgets::{Button, Label, Panel}};
use denise_arrange::{Arrange, Flow, Sizing};

let mut ui: Ui<Void> = Ui::new(Size::new(400, 200), theme::DARK);
let root = ui.root();
let title = ui.add(root, Label::new("Settings"), Rect::ZERO).unwrap();
let spacer = ui.add(root, Panel::default(), Rect::ZERO).unwrap();
let save = ui.add(root, Button::<Void>::inert("Save"), Rect::ZERO).unwrap();

let mut arrange = Arrange::new(Flow::Row);
let row = arrange.root();
arrange.set_padding(row, 8);
arrange.set_gap(row, 8);

arrange.node(row, title, Sizing::Hug);       // as wide as its text
arrange.node(row, spacer, Sizing::Flex(1));  // everything left over
arrange.node(row, save, Sizing::Hug);        // as wide as its label

arrange.apply(&mut ui, Rect::new(0, 0, 400, 44));

assert_eq!(ui.layout(title).unwrap().x, 8);
assert_eq!(ui.layout(save).unwrap().right(), 392);
```

| | |
|---|---|
| `Sizing::Fixed(n)` | This many pixels. What every node is without this crate. |
| `Sizing::Flex(w)` | A share of what is left, in proportion to the other weights. |
| `Sizing::Hug` | Whatever the widget says it wants, through `Widget::measure`. |

The cross axis is not a choice: a child fills it. A widget that wants to sit in
the middle of a taller row gets `Fixed` and a margin, which is arithmetic somebody
can read.

## Rows, columns and layers

`Flow::Row` and `Flow::Column` are the two axes. `Flow::Layer` puts every child in
the whole content box, one on top of another — a badge over an avatar.

It is **not** called a stack, because `Ui::set_stack` already means top-to-bottom
and two meanings of one word in one workspace is a trap.

Containers nest with `group`, and a nested container may *be* a node of the tree —
a `Panel` holding a row of buttons — in which case it gets the container's
rectangle and its children are placed inside it.

## Two passes, because one is not enough

Measure, then arrange. An `Alert` has no height until it knows the width its text
wraps to, and a `Rating` has no width until it knows how tall its stars are — so a
hugging child is measured against the cross axis the container can already
promise, and only then is the leftover shared out.

That is what `Offer` and `Measured` in `denise-ui` are for, and why they are per
axis: a widget with an opinion about one axis and none about the other says so
rather than inventing the second.

## What it will not do

No wrapping. No per-item alignment or baselines. No grid. No min/max, and so no
shrinking rules. No absolute children — a node not in an arrangement keeps the
rectangle it was given, which is how every node works without this crate.

Flexbox was the other candidate and costs materially more code and more per-pass
work, in a crate whose entire justification is that a Pi 3 A+ does not pay for it
unless asked.

## When it runs

When you call `apply`, and never otherwise. There is no invalidation to get wrong
and no per-frame cost: an application that has not called it has not laid anything
out, and one that wants layout on resize calls it where it already handles the
resize.

`no_std + alloc`, like everything below `denise-ui`.
