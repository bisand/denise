# denise-arrange — content-driven sizing, for the applications that ask

A design note, written before the crate, because [#112] exists to settle what the
crate is before anybody writes it. Nothing here is implemented yet. What it
records is the model, what was given up to get it, what it costs the crates that
already exist, and the part that turned out to be the real work.

[#112]: https://github.com/bisand/denise/issues/112

## The position this has to respect

From the README, and it is a claim rather than an apology:

> **No layout engine.** Nodes are positioned with explicit rectangles relative to
> their parent, which is what a fixed-resolution panel wants.

A kiosk at a known resolution does not want a solver, and paying for one in binary
size and in per-frame cost on a Pi 3 A+ would be a real loss. That stays true, and
this crate is how it stays true: `denise-ui` gains no solver, a new crate has one,
and an application that does not depend on the new crate links none of it.

Two smaller answers already landed and are not this:

| | | |
|---|---|---|
| **Resize** | A window dragged, a panel turned to portrait | `anchor=` and `dock=` ([#110]) — one derived rectangle per child, in the reflow pass the tree already runs. |
| **Scale** | The same design at more pixels | `scaling=` ([#111]) — one multiply on the way in, in `denise-forms`. |
| **Content-driven sizing** | A label as wide as its text; a row that grows with what is in it; a dialog that sizes to its contents | Nothing. This note. |

[#110]: https://github.com/bisand/denise/issues/110
[#111]: https://github.com/bisand/denise/issues/111

The third is different in kind from the first two. Anchoring and scaling both
derive a rectangle from a rectangle. Content-driven sizing has to **ask the
content how big it is**, and that is a measure pass, and a measure pass is a
layout engine however small it is.

## What the survey found, and why it changes the plan

The issue assumes the crate's job is to make queries that already exist:

> Widgets already offer `preferred_width`/`preferred_height` as *queries the
> application makes* … This crate is the thing that makes those queries.

That is half true, and the missing half is most of the work.

**Twelve of twenty-six widgets offer a query at all.** The fourteen that offer
nothing are `avatar`, `carousel`, `collapse`, `divider`, `image`, `label`,
`panel`, `progress`, `radial-progress`, `select`, `slider`, `spinner`,
`text-input` and `video`.

`label` is on that list. *A label as wide as its text* is the issue's own first
example of what content-driven sizing means, and today it cannot be answered at
all.

**The twelve that do answer do not agree on the question.** These are inherent
methods on concrete types, each grown when one example needed it:

| | |
|---|---|
| `badge`, `button`, `tabs`, `list`, `tree` | `(engine)` — free measure |
| `checkbox`, `radio-group`, `toggle` | `(theme, engine)` — the theme decides part of the size |
| `list`, `radio-group`, `timeline`, `tree` | `preferred_height(theme)` — no text measured |
| `table` | `(theme, rows)` — a count the widget does not hold |
| `alert` | `preferred_height(engine, width)` — **height for a width** |
| `rating` | `preferred_width(height)` — **width for a height** |

**`Widget` has no measure method.** Paint, event, hit-test, focus — nothing about
size. So a crate holding `NodeId`s cannot ask a node how big it wants to be, not
because the answer is missing but because there is no door. Today's only caller,
`examples/gallery`, measures widgets it is holding **before** it inserts them:

```rust
let badge = Badge::new(text).with_role(role).with_style(self.small);
let bw = badge.preferred_width(self.ui.text_mut());
self.add(s, badge, Rect::new(x, 104, bw, bh));
```

That works because `badge` is a local. A layout pass over a built tree has no
local — it has a `NodeId` — and `widget.preferred_width(ui.text_mut())` cannot
compile anyway, because both halves borrow the same `Ui`.

So the crate's real prerequisite is a **uniform measure protocol**, and that lands
in `denise-ui` before any arranging is possible.

## The model: rows, columns and layers

Containers nest. Each child is a fixed size, a flex weight, or hugging.
Containers have padding and a gap.

Three flows: `Row`, `Column`, and `Layer` — every child in the whole content box,
one on top of another, which is a badge over an avatar. This note first called
that third one a *stack*, and it is not: `Ui::set_stack` already means
top-to-bottom, and two meanings of one word in one workspace is the same trap
`Fit` would have been.

```rust
let mut arrange = Arrange::new(Flow::Row);
let row = arrange.root();
arrange.set_padding(row, 12);
arrange.set_gap(row, 8);

arrange.node(row, title, Sizing::Hug);      // as wide as its text
arrange.node(row, spacer, Sizing::Flex(1)); // everything left over
arrange.node(row, save, Sizing::Hug);

arrange.apply(&mut ui, toolbar);            // one set_layout per node
```

The arena takes a parent the way [`Ui::add`] does, rather than the chained
builder this note first sketched: containers nest, so a child needs to say which
container it is in, and one shape for that is better than two. `Arrange::group`
adds a nested container, and it may *be* a node of the tree — a `Panel` holding a
row of buttons — in which case that node gets the container's rectangle and its
children are placed inside it.

[`Ui::add`]: https://docs.rs/denise-ui/latest/denise_ui/struct.Ui.html#method.add

Three ways a child can be sized, and that is the whole vocabulary:

| | |
|---|---|
| **fixed** | This many pixels. What every node is today. |
| **flex(n)** | A share of what is left after the fixed and hugging children have taken theirs. |
| **hug** | Whatever the widget's own measure says. The one that needs the protocol above. |

### What was given up

Written down because a note that only lists what a design does is an
advertisement.

- **No wrapping.** A row that runs out of room clips or squeezes; it does not
  become two rows. Wrapping needs a second arrange pass whose input is the first
  pass's output, and every panel this toolkit has met has a known width.
- **No per-item alignment or baselines.** Children fill the cross axis. A widget
  that wants to sit in the middle of a taller row gets a fixed size and a margin,
  which is arithmetic somebody can read.
- **No grid.** Two axes of distribution is a solver with a name; a row of columns
  covers the cases a panel actually has.
- **No min/max constraints, and so no shrinking rules.** A flex child gets its
  share. If the share is too small the widget clips, which it already does.
- **No absolute or floating children.** A node not in a container keeps the
  rectangle it was given, which is how every node works today.

Flexbox was the other candidate and is what people know. It costs wrap,
justify/align, and grow/shrink/basis — materially more code and more per-pass work
in a crate whose entire justification is that a Pi 3 A+ does not pay for it unless
asked. Grid was never a candidate.

### Two passes, because one is not enough

Measure, then arrange.

The reason is `alert`, and it is not an edge case: `preferred_height(engine,
width)` is **height for a width**, because wrapped text has no height until it has
a width. A single pass that asked every child for a size and then placed it would
have to ask the alert its height before deciding its width, and would get an
answer to the wrong question.

So: pass one walks the container and asks each child to measure against the
constraint the container can already promise — on a row, the cross axis; on a
column, the main axis. Pass two distributes what is left and writes the
rectangles. `rating`'s `preferred_width(height)` is the same shape mirrored, which
is a second witness that the shape is real rather than one widget being awkward.

## What it costs the crates that already exist

`denise-ui` gains **one trait method and one `Ui` method**. Not a solver — a door.
**Built**, in [#147].

[#147]: https://github.com/bisand/denise/pull/147

```rust
pub trait Widget<M> {
    /// How big this widget would like to be, given what the caller can promise.
    fn measure(&self, ctx: &mut MeasureCtx<'_>, offered: Offer) -> Measured {
        Measured::NOTHING
    }
}

impl<M> Ui<M> {
    pub fn measure(&mut self, id: NodeId, offered: Offer) -> Measured;
}
```

`MeasureCtx` carries the theme and the text engine and nothing else — no bounds,
because a widget asked how big it wants to be must not answer with how big it
already is; no state, because a hovered button is not a wider button.
`Ui::measure` exists because the borrow has to be resolved inside `Ui`, where both
halves live.

**`Offer` and `Measured` are per axis, and this note's first sketch was wrong to
say `Option<Size>`.** The widgets are per axis and always were: an `Alert` has an
opinion about its height and none about its width, a `Table` about its height and
none about its width, a `Rating` about its width and none about its height. A
single `Option<Size>` would force a widget with one opinion to invent the other,
and an invented size is worse than no size — no size, the caller notices and
decides. `Offer` is likewise not a *constraint*: there is no minimum, no maximum
and nothing to satisfy, only the one fact a widget may need in order to answer at
all.

**This does not make the tree a layout engine.** `design.md` draws the line
precisely, and the line survives:

> An intrinsic-size *protocol* is one where the tree asks every widget how big it
> wants to be and then places it. Here the *application* asks, does its own
> arithmetic, and passes a rectangle.

After this change the tree still never calls `measure`. Nothing in `denise-ui`
consumes it. It is the same query the twelve widgets already offer, given one
signature and one door, so that a caller who is not holding the concrete type can
ask it. The caller is still the application — or a crate the application chose to
depend on.

The twelve inherent `preferred_*` methods stay, because they are a nicer thing to
call when you *are* holding the widget, and `examples/gallery` is that caller.
They become thin wrappers over the same arithmetic rather than a second copy of
it.

The fourteen widgets with no answer get one where there is an honest one to give.
Four did: `label` (as wide as its text — the case that could not be answered at
all), `select` (as wide as its longest option, so a dropdown does not clip the
thing it exists to show), `text-input` and `collapse` (a height each, and no view
about their width).

The rest keep `Measured::NOTHING`, and that is the finding rather than the
shortfall: an `image`, a `video`, a `panel`, an `avatar`, a `progress`, a
`spinner`, a `radial-progress`, a `carousel` and a `slider` are all **whatever
rectangle they are given**. Their paint code derives every number from `bounds`.
Inventing a size for them would have been a number a caller then silently obeyed.

## The questions the issue asked

**When does it run?** On demand. `arrange.apply(&mut ui, root)` is a call an
application makes, exactly where it would have done the arithmetic by hand. It is
not hooked to resize, and the tree does not know the crate exists — which is what
keeps it optional in fact and not only in the dependency graph.

**Does it cost anything when nothing changes?** Nothing, because nothing runs.
There is no invalidation to get wrong: an application that does not call `apply`
has not laid anything out. An application that wants layout on resize calls
`apply` when it handles the resize, which is one line in the place that already
knows.

**`no_std + alloc`?** Yes. `denise-ui` is `#![cfg_attr(not(feature = "std"), no_std)]`
and everything below it is; a layout crate that needed `std` would be the first
thing in the stack a bare-Linux build could not use. Containers and children are
`Vec`s in an arena, which is `alloc`, which is what `denise-ui` already requires.

**Does the form file get to say any of it?** **No, and not later without a version
story.** A `.dform` describes rectangles because the toolkit draws rectangles. A
file that described a row with flex children would build into a tree that an
application not linking `denise-arrange` renders as a heap of overlapping nodes —
a file that silently means different things depending on the reader's dependency
graph, which is the worst property a format can have. If it is ever worth it, the
form node says so and the engine refuses to build it without the crate present,
and that is a schema change with its own number.

## What lands, and in what order

1. **This note.** It is the deliverable that lets the rest be checked against
   something.
2. **The measure protocol in `denise-ui`.** ✅ `Widget::measure`, `Ui::measure`,
   and answers for the sixteen widgets that have one. Useful on its own: an
   application doing its own arithmetic can ask one question instead of twelve
   differently-shaped ones.
3. **`denise-arrange`.** ✅ The crate, `no_std + alloc`, its own README, doc
   examples compiled by `cargo test --doc`.
4. **An example.** ✅ [`examples/arranged`](../examples/arranged) is the same
   screen laid out both ways, and a test asserts they land in **exactly the same
   rectangles** at three different sizes. Rendered, the two are byte-identical —
   which is the claim stated as strongly as it can be: the crate computes what
   the application would have computed.

Steps 2 and 3 were separately useful and separately reviewable, which is why they
were separate. Step 2 is also the one that touched a published crate, so it was
the one to get wrong slowly.

## What this will never be

Not a constraint solver. Not a style system. Not a thing the tree calls. If an
application wants a label as wide as its text, this computes the width and calls
`set_layout`, which is what the application would have done — and if the
application would rather do it itself, it still can, and links nothing.
