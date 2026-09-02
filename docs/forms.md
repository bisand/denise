# The form file

A form in Denise is Rust: a `Ui<M>`, a tree of `ui.add(parent, widget, rect)`
calls, and a `match` over the messages that come back. That is the toolkit and it
is not going away. This document is about the *other* way to say the same thing —
a text file a visual designer writes, a person edits, `git diff` reads, and
`denise-forms` loads at runtime.

This is the format's design note and its version 1 specification. It is long
enough to write a form by hand from, which is the point: a format that needs its
editor is not hand-editable, whatever its authors claim.

Start with [design.md](design.md) if you want to know why the toolkit is shaped
the way it is. Read [the designer's issue](https://github.com/bisand/denise/issues/106)
for what is being built on top of this.

---

## Why there is a file

Three consumers, and each one constrains the format differently.

**The designer** ([#106](https://github.com/bisand/denise/issues/106)) reads a
file, shows it on a canvas, and writes it back after every drag. It needs a shape
it can edit *surgically* — moving one button must not rewrite the document.

**The engine** (`denise-forms`, [#86](https://github.com/bisand/denise/issues/86))
parses a file into a `Ui<M>` at runtime, or at startup from an `include_str!` on a
board with no filesystem worth reading. It needs errors that name a line, a
column, and what was expected, because the file is something a person typed.

**The person** needs to open it in an editor, understand it without a legend, fix
a rectangle, and not lose their comments when the designer next saves. Delphi's
`.dfm` got this right and it is most of why that designer was trusted. Most
designers since did not, and are not.

The three pull in the same direction more than they conflict, which is what makes
one format possible.

## What the format has to do

In order, because the later ones are cheap once the earlier ones hold.

1. **Be hand-editable.** Comments. A layout somebody would write by hand. No
   noise a tool needs and a reader does not — no GUIDs, no `<?xml`, no
   `"$type"`. A property at its default is not written at all.

2. **Diff well.** One node per line where the node is small, one property per
   line where it is not; stable key order; nodes in tree order. Moving a node on
   the canvas produces a one-line diff, and a reviewer can see it is a move.

3. **Round-trip.** Open in the designer, save, `git diff` is empty — including
   comments, including spacing, including a property the designer has never heard
   of. This is the hard one, and it decides the format on its own: see
   [round-tripping](#round-tripping-is-the-requirement-that-picks-the-format).

4. **Be a tree.** A `Panel` has children whose rectangles are relative to it.
   That is the scene graph's model exactly, and the file nests the same way, so
   there is no mapping to get wrong.

5. **Carry a version.** So an engine can refuse a file from the future in a
   sentence rather than a panic, and read one from the past.

6. **Type the values the widgets actually take.** Geometry, theme *roles* rather
   than colours, enums for `Align` and `Fit` and the rest, and a *name* where the
   application has a message.

## The choice: KDL

| | Comments | Nested trees | Round-trips | Notes |
|---|---|---|---|---|
| **KDL** | ✅ | ✅ — it *is* a node tree | ✅ in the library | chosen |
| RON | ✅ | ✅ | ✗ via serde | the fallback |
| TOML | ✅ | awkward past two levels | ~ `toml_edit` | |
| JSON | ✗ | ✅ | ✗ | no comments ends it |
| YAML | ✅ | ✅ | ~ | the footguns are documented elsewhere |
| XML | ✅ | ✅ | ~ | see [below](#and-why-not-xml) |

A form is a tree of named nodes, each with a little positional content and a bag
of typed properties, some of which have children. That sentence is the KDL data
model with nothing bolted onto it. Every other candidate needs a convention laid
over it to mean the same thing — `[[widgets]]` arrays, a `children:` key, a
`kind` discriminator — and a convention is a thing that can be got wrong, in the
writer and in the reader separately.

```kdl
form "Hello" version=1 kind=screen width=460 height=260 theme=dark {
    panel name=card x=16 y=16 w=428 h=228 {
        label "Hello, Denise"      x=20 y=18 w=388 h=28 size=22
        label "What is your name?" x=20 y=58 w=388 h=20 size=16

        // Enter submits, so a keypad-only panel never needs the button.
        text-input name=who x=20 y=82 w=388 h=34 placeholder="your name" on-submit=greet size=16 focus=#true

        button "Greet" x=20 y=128 w=110 h=34 role=primary on-press=greet size=16

        // Filled in by the application from what was typed.
        label "" name=greeting x=20 y=176 w=388 h=24 size=16
    }
}
```

That is [`forms/hello.dform`](../forms/hello.dform), and it is
[`examples/hello`](../examples/hello) — the same tree, minus the arithmetic that
centres the card, which is the [no-layout-engine boundary](#what-the-format-will-not-do)
showing exactly where it is.

The extension is `.dform`. One form per file.

### Round-tripping is the requirement that picks the format

Requirement 3 is not a property of a *syntax*. Every format in that table can be
written back out. It is a property of the **parser**, and specifically of whether
the parser hands you a document or a value.

A serde-shaped parser gives you a value: `Form { nodes: Vec<Node> }`. The
comments are gone by then, the key order is gone, the spacing is gone, and the
property your struct has no field for is gone. Serialising that back produces a
file that *means* the same thing and *is* a different file. Open, save, and the
diff is the whole document — which is how a designer teaches people not to open
their files in it.

[`kdl`](https://docs.rs/kdl) (6.7, Apache-2.0, MSRV 1.95 — the workspace's own)
is document-oriented: `KdlDocument` keeps the comments, the whitespace and the
entry order, lets nodes be edited in place, and `to_string()` gives back what was
parsed. Requirement 3 is then most of the way met by the library rather than by
our discipline, which is the only way it stays met.

Most of the way, not all of it. Fuzzing (#104) found kdl dropping the trivia
between a closing brace and the next node — trailing whitespace, and a comment
written on the brace's line — so `Form::parse` puts those bytes back and then
**checks**: it writes the document out, compares it to the source, and refuses a
file it cannot reproduce rather than accepting one that would lose bytes on the
first save. A guarantee this load-bearing is worth verifying on every file
rather than believing about a dependency.

The consequence for the designer is architectural, and it is the reason this note
comes before any code: **the designer's model is the document.** It does not own
a struct that it serialises on save. Every canvas edit is a targeted edit on the
`KdlDocument`, and the tree on screen is derived from it.
[#88](https://github.com/bisand/denise/issues/88) is where that gets tested.

`kdl` also carries `miette` diagnostics with real spans, which is most of
requirement "errors that name a line and a column" already done.

### And why not RON

RON was the close second and would be a perfectly good `.dform`. It is Rust's own
shape — `Role::Primary` is spelled `Primary`, enums need no convention at all,
and `ron` is a dependency this workspace would find unremarkable.

Two things decided against it. It has no document-preserving parser, so
requirement 3 would be ours to build and ours to keep working. And its tree is a
struct tree, so a node is `Widget(kind: Button, children: [...])` — every node
carries the word `children` and a pair of brackets that KDL gets from the syntax.
For a file whose whole job is to be read by people, that is a real cost paid on
every line.

If `kdl` ever becomes untenable, RON plus a hand-written formatter is the exit,
and the schema below survives the move nearly unchanged.

### And why not XML

Because the audience for this is people who remember `.dfm` and `.designer.cs`
fondly and XAML less so, and because a format that needs closing tags to say
`label` costs three times the characters for the same tree. `.dfm` itself — the
Delphi format all of this is descended from — is much closer to KDL than to XML,
and it was not an accident.

## The v1 schema

### The document

One top-level node, `form`. Anything else at the top level is an error.

```kdl
form "Title" version=1 kind=screen width=800 height=480 { ... }
```

`version` is required and is the **major** schema version. An engine reads a file
whose `version` it knows and refuses one it does not, by number, in a sentence.
Within a major version, properties may be added; an engine older than the file
will report the added property as unknown, which is the same error a typo
produces and has the same fix — the message says both.

Encoding is UTF-8. KDL v2 syntax
([spec](https://github.com/kdl-org/kdl/blob/main/draft-marchan-kdl2.md)); v1 KDL
is not accepted.

### The `form` node

| | Type | Default | |
|---|---|---|---|
| *(first argument)* | string | — | The form's title. Required; shown in a window's title bar, and the designer's name for it. |
| `version` | integer | — | Required. `1`. |
| `kind` | [form kind](#form-kinds) | `screen` | What this form is for. |
| `width` `height` | integer | — | Required, logical pixels. What the form was designed at. |
| `theme` | `dark` `light` `high-contrast` | `dark` | A built-in theme. |
| `background` | [role](#roles) | `base-100` | The surface the form is drawn on. |
| `scaling` | `none` `proportional` `stretch` | `none` | Whether this form may be drawn at a size other than the one it was designed at. See [Responsiveness](#responsiveness). |
| `name` | identifier | — | Optional. Used by the typed layer ([#101](https://github.com/bisand/denise/issues/101)) to name what it generates. |

#### Form kinds

The kind is a statement about how the form is *shown*, which the toolkit expresses
three different ways depending on the machine. The engine reports the kind; it
does not open anything. [#90](https://github.com/bisand/denise/issues/90) owns
what the designer does with each.

| `kind` | Maps to | Extra properties |
|---|---|---|
| `screen` | the root tree of a `Ui`, a panel's whole surface | — |
| `window` | a `denise-winit` `WindowConfig` | `resizable` (bool, `#true`), `min-width`, `min-height` (integer) |
| `dialog` | `Ui::push_scene` on a panel; a `Modality::Modal` window on a desktop | `dim` (integer `0`–`255`, `160`) |
| `drawer` | `Ui::push_drawer` | `side` ([side](#other-enums), `before`), `extent` (integer, **required**) |
| `shelf` | `Ui::push_shelf` | `side` ([side](#other-enums), `below`), `extent` (integer, **required**) |
| `fragment` | a subtree with no root of its own, for reuse — Delphi's *Frame* | — |

`width` and `height` are still required for a drawer or a shelf: `extent` is how
far it comes in, and the other axis is the surface it comes in over, which the
designer needs in order to draw it.

**A kind's extra properties are that kind's.** `resizable` on a screen is not a
property with no effect; it is a window's property on something that is not a
window, and it is refused with the same error a typo gets — which is what the
paragraph above promises for every node, and the `form` node is a node. There is
a small file for each kind next to the reference form:
[`window`](../forms/window.dform), [`dialog`](../forms/dialog.dform),
[`drawer`](../forms/drawer.dform) and [`shelf`](../forms/shelf.dform).

### Every node

A widget node's name is its **kind**. It may take one positional argument — its
primary text, where it has one — and then properties. These apply to all of them:

| | Type | Default | |
|---|---|---|---|
| `name` | identifier | — | This node's identifier, unique within the form. Only needed if something looks the node up: the application (`built.node("who")`), the typed layer, or another form. A label nobody reads back needs no name. |
| `x` `y` `w` `h` | integer | — | **Required.** The node's rectangle, in logical pixels, *relative to its parent*. |
| `visible` | bool | `#true` | `Ui::set_visible`. |
| `enabled` | bool | `#true` | `Ui::set_enabled`. |
| `z` | integer | `0` | `Ui::set_z`. Siblings otherwise draw in file order. |
| `tooltip` | string | — | `Ui::set_tooltip`. |
| `scroll` | bool | `#false` | `Ui::set_scrollable` — this node is a viewport for children larger than it. |
| `stack` | integer | — | `Ui::set_stack`: place children top to bottom with this spacing. A stacked container's children still declare `y`; the tree overwrites it. Containers only. |
| `focus` | bool | `#false` | This node has the caret when the form opens. At most one per form. |
| `anchor` | names | `"left top"` | Which of the parent's edges this node keeps its distance from, space separated, from `left` `top` `right` `bottom`. Both edges of an axis and it stretches; neither and it stays centred between them. See [Responsiveness](#responsiveness). |
| `dock` | `top` `bottom` `left` `right` `fill` | — | Take an edge of what is left of the parent, across its full width or height. Docked nodes are placed in file order, each from what the ones before it left, and everything undocked is placed in what remains. |

There is no `id` distinct from `name`, no `class`, and no style attribute. A
widget's appearance comes from its [role](#roles) and the theme, and nothing else.

### Tab order is file order

**Settled, and there is no `tab-index`.** Tab walks the file top to bottom,
depth first — everything inside a panel comes between the thing before it and the
thing after it — and Shift-Tab walks it back. A form is a loop, so the last stop
leads to the first.

That is the whole rule, and it is the reason there is no second number: the file
already has an order, a person chose it, and [saving keeps it](#hand-editing). A
`tab-index` would say the same thing a second way, and the two would drift the
first time somebody edited one of them.

**To change the order, move the node in the file.** Nothing on the canvas moves
when you do — every rectangle is absolute — so this is a reordering and not a
relayout. The designer's [tab order mode](../tools/designer/README.md#tab-order)
is that, with the numbers drawn on.

Three things are not stops: a node that cannot take focus at all (a `label`, a
`divider`), a node with `enabled=#false`, and a node with `no-focus=#true`, which
is a widget that takes no focus *and costs none* — a repeat button beside a video
is not somewhere Tab should stop.

One coupling worth knowing: **`z=` sorts siblings, and the tab order is the
sibling order**, so raising a node in front of its siblings also makes it a later
stop. That is defensible — a thing drawn on top is reached after what it covers —
and it is the one way tab order stops being *file* order. It has a test.

`focus=#true` on one node is where the caret starts. With nothing said, nothing is
focused, because a form that grabbed the caret unasked would take it from whatever
put the form up.

All of this is asserted headlessly in `denise-forms/tests/focus.rs`, including the
reference form's twenty stops in order.

### Geometry

Four integers, not one packed string. `x=20 y=44 w=388 h=34` costs a few
characters over `rect="20 44 388 34"` and buys real KDL numbers: an error points
at the number rather than at the string containing it, an editor treats them as
numbers, and nothing has to parse inside a value. Delphi wrote `Left`, `Top`,
`Width`, `Height` on four lines and nobody ever complained about it.

Negative values are allowed and mean what they say — a node partly outside its
parent, clipped by it. That is the rendered truth and the designer draws it.

### Collections

Six widgets hold **content** as child nodes rather than as a value: a `select`
holds `option`s, a `table` holds `column`s and `row`s. That is not the same as
holding *children* — dropping a button on a `select` has missed, and dropping
one on a `panel` means it, which is what `owns_children` answers.

Each collection is one of two things, and which one it is decides who may edit
it and whether it ships:

| child | in | | |
|---|---|---|---|
| `option` | `select`, `radio-group` | **real** | A dropdown's choices are the choices. |
| `tab` | `tabs` | **real** | A form's sections are the form's. |
| `item` | `list`, `tree` | **real** | A navigation list is content, not sample data. |
| `column` | `table` | **real** | A table's columns are its *shape*; its rows are not. |
| `picture` | `carousel` | **real** | A kiosk's slideshow is the slideshow. |
| `row` | `table` | **placeholder** | Never the records — the application supplies those. |
| `event` | `timeline` | **placeholder** | Likewise. |

**Real** content is the widget's own and is edited where it lives. Each is a
`PropertyKind::List` property named after the child node — a property called
`option` *is* the `option` nodes under it — so the designer's inspector shows it
as a run of fields with the controls that add, remove and reorder.

The items are edited **one node at a time**: retyping one writes that one's
argument, and adding, removing or reordering is an insert, a removal or a move.
Nothing rewrites the block, which is why a comment written above the third
option is still above the third option afterwards, and why every one of those is
a single undo step that restores the file to the byte.

A collection whose item is more than one string is real content that this
editor cannot yet edit: a `row` has an argument per column and a `picture` has a
path rather than an argument. They are read and written by the engine as they
always were.

**Placeholder** content is what the *designer* needs to show the widget
convincingly — four rows of names so a table looks like a table. It goes in a
`design { … }` block on the widget, and **every build but a designer's skips
it**, so it never reaches a panel:

```kdl
table name=records x=8 y=8 w=384 h=140 {
    column "First" width=120
    column "Last" flex=#true

    design {
        row "Ada" "Lovelace"
        row "Grace" "Hopper"
    }
}
```

`Form::build` and `Form::build_scaled` come up with the columns and no rows;
`Form::build_with_design` is the designer's, and is the only one that reads the
block. Each is a `PropertyKind::Placeholder` property, which differs from a
`List` in where it is written and who builds it and not at all in what an
inspector does with one.

A `row` or an `event` written *outside* a `design` block is an **error**, not
something quietly ignored — the same choice, for the same reason, as an unknown
property. Ignoring it would mean a form that looks right in the designer and
comes up empty on a panel, which is the failure nobody notices until the panel
is on a wall.

The block is narrow on purpose: it holds that widget's placeholder collections
and nothing else. A widget hidden inside one would be something the file
describes and the engine never builds, which is a larger idea than this needs
and a worse one to meet by accident.

One consequence worth knowing: a `table` built without its rows has nothing to
select, so it is not a tab stop until the application supplies records. The
designer sees a stop there and a panel does not, and that is the placeholder
data being exactly as absent as it should be.

### Value types

#### Roles

The twenty theme roles, kebab-cased from `denise::Role`:

`base-100` `base-200` `base-300` `base-content` `primary` `primary-content`
`secondary` `secondary-content` `accent` `accent-content` `neutral`
`neutral-content` `info` `info-content` `success` `success-content` `warning`
`warning-content` `error` `error-content`

**There are no colour literals in a form file**, with one exception noted at
[`video`](#video). A form says `role=primary`; the theme says what that is
today, and a theme swap is one call. A file that could say `#89B4FA` would be a
file that survives a theme change looking wrong, and the toolkit went to some
trouble to make that impossible.

#### Other enums

| Type | Values | From |
|---|---|---|
| radius | `selector` `field` `box` | `denise::Radius` |
| align | `start` `center` `end` | `denise_ui::widgets::Align` |
| orientation | `horizontal` `vertical` | `denise_ui::widgets::Orientation` |
| fit | `fill` `contain` `cover` `center` | `denise_ui::widgets::Fit` |
| presence | `online` `offline` `busy` | `denise_ui::widgets::Presence` |
| side | `above` `below` `before` `after` | `denise_ui::Side` |

#### Text size

`size` is a text size in logical pixels, an integer, on every widget that draws
text. Omitted, it is **16** — the widgets' own built-in default — with `badge`
the single exception at 14. It is not a theme token: text size is the one piece
of geometry a theme does not own, because a form decides what is a heading and
the theme cannot.

A form cannot select *which* font a widget uses in v1. Fonts are registered at
runtime and identified by a `FontId` the file cannot know; a name-to-`FontId`
resolver is the obvious addition and is not in version 1 because nothing in the
repository needs two fonts in one form yet.

#### Messages

A widget does not run a callback. It holds a value of the application's type and
emits it, and a form file cannot hold a value of a type it has never seen — so it
holds a **name**, and the application maps names to its own enum:

```kdl
button "Greet" x=20 y=128 w=110 h=34 on-press=greet
```

```rust
form.build(&mut ui, root, |name, payload| match (name, payload) {
    ("greet", Payload::None) => Some(Message::Greet),
    ("set-notify", Payload::Bool(on)) => Some(Message::Notify(on)),
    _ => None,
})?;
```

An unknown name is an error at load, naming the name — not a button that silently
does nothing, which is the failure mode every string-keyed UI format has and the
reason people distrust them.

**The payload matters and is easy to miss.** `Button` holds an `M`. `Checkbox`
holds a `fn(bool) -> M`. `List` holds a `fn(usize) -> M`. `Slider` holds a
`fn(f32) -> M`. So a resolver that is only `Fn(&str) -> Option<M>` cannot build a
checkbox, and the payload column in the widget tables below is what each message
property needs:

| Payload | Widgets |
|---|---|
| none | `button` `text-input` `select` |
| bool | `checkbox` `toggle` `collapse` |
| index | `list` `table` `radio-group` `tabs` `carousel` |
| number | `slider` `rating` |

Message names are passed to the resolver verbatim. The convention in this
repository is kebab-case, and the typed layer
([#101](https://github.com/bisand/denise/issues/101)) maps kebab to the
PascalCase variant it generates; `on-press=Greet` is equally valid and arrives as
`"Greet"`.

#### Assets

`src` is a path **relative to the form file**, never to the working directory —
so a form and its pictures move together and a kiosk's current directory is not
part of the contract.

The engine does not decode images; `denise-forms` does not depend on
`denise-image`. The application supplies a loader, which is also how a board that
compiles its pictures in hands over a lookup table instead of touching a disk.

#### Collections

Options, rows, columns and items are **child nodes**, not packed strings — one
per line, so adding one is a one-line diff and reordering is visible as a
reordering. Each collection node is listed with the widget that owns it.

Whether a given collection is real data or a design-time placeholder is settled
per widget — a `select`'s options are the real ones, a `table`'s rows never are —
and the two are written in different places: real content as children of the
node, placeholder content inside a
[`design` block](#collections) the engine skips. So an application replaces
nothing at startup; it supplies the records it always had.

---

## The widgets

Twenty-six widgets and one container. Every property is optional unless marked
**required**; every default below is the widget's own, and a property at its
default is not written to the file.

### Containers

#### `panel`

The only widget with children. No positional argument.

| | Type | Default | |
|---|---|---|---|
| `fill` | [role](#roles) | `base-200` | Surface colour. `fill=none` for no fill. |
| `border` | [role](#roles) | `base-300` | Border colour. `border=none` for no border. |
| `border-width` | integer | `1` | |
| `radius` | [radius](#other-enums) | `box` | |
| `backdrop` | bool | `#false` | This panel absorbs presses rather than letting them fall through, and leaves the focus where it is. What the sheet under an on-screen keyboard is: a finger landing in the gap between two keys must not dismiss it. Not a dim — dimming is `kind=dialog`'s `dim`. |

Any node may in fact carry children in the file; only `panel` is *meant* to, and
the engine rejects children on a widget that cannot lay them out.

### Text

#### `label`

| | Type | Default | |
|---|---|---|---|
| *(first argument)* | string | `""` | The text. |
| `role` | [role](#roles) | `base-content` | |
| `align` | [align](#other-enums) | `start` | Horizontal. |
| `valign` | [align](#other-enums) | `center` | Vertical. |
| `size` | integer | `16` | |

#### `badge`

| | Type | Default | |
|---|---|---|---|
| *(first argument)* | string | `""` | **Required in practice.** The text. |
| `role` | [role](#roles) | `primary` | |
| `size` | integer | `14` | |

#### `alert`

| | Type | Default | |
|---|---|---|---|
| *(first argument)* | string | — | **Required.** The message. |
| `role` | [role](#roles) | — | **Required** — an alert with no status is a label. |
| `icon` | string | — | A single character drawn before the text. |
| `size` | integer | `16` | |

#### `divider`

| | Type | Default | |
|---|---|---|---|
| *(first argument)* | string | — | An optional label sitting in the rule. |
| `orientation` | [orientation](#other-enums) | `horizontal` | |
| `role` | [role](#roles) | `base-300` | |
| `size` | integer | `16` | Only a labelled divider draws text. |

### Input

#### `button`

| | Type | Default | |
|---|---|---|---|
| *(first argument)* | string | `""` | The label. |
| `on-press` | message (none) | — | Omitted, the button is inert — it draws and does not emit, which is `Button::inert`. |
| `role` | [role](#roles) | `primary` | |
| `radius` | [radius](#other-enums) | `field` | |
| `size` | integer | `16` | |
| `corner` | string | — | A small legend in the corner, as the keyboard's globe key carries its layout. |
| `no-focus` | bool | `#false` | The button never takes focus, so pressing it does not steal the caret from a field. What an on-screen keyboard's keys are. |
| `repeat-delay` | integer | — | Milliseconds held before the press repeats. Both repeat properties or neither. |
| `repeat-interval` | integer | — | Milliseconds between repeats. |
| `watch-hold` | bool | `#false` | Report how long the button has been held, for a long-press. |

`icon` is not settable from a form file in version 1. `Button::with_icon` takes a
`&'static Icon` — a vector shape compiled in — and the only ones that exist live
in `denise-keyboard`. A named registry of built-in icons is what this property
needs first, and there is nothing yet to name.

#### `text-input`

No positional argument: a field's text is its state, not its identity.

| | Type | Default | |
|---|---|---|---|
| `text` | string | `""` | Initial contents. |
| `placeholder` | string | `""` | Shown while empty. |
| `on-submit` | message (none) | — | Emitted on Enter. |
| `max-chars` | integer | `256` | |
| `password` | bool | `#false` | |
| `size` | integer | `16` | |

#### `checkbox`, `toggle`

Identical properties; they differ only in how they draw.

| | Type | Default | |
|---|---|---|---|
| *(first argument)* | string | `""` | The label. |
| `checked` | bool | `#false` | |
| `on-change` | message (bool) | — | Omitted, the widget is inert. |
| `role` | [role](#roles) | `primary` | |
| `size` | integer | `16` | |

#### `radio-group`

| | Type | Default | |
|---|---|---|---|
| `selected` | integer | `0` | Index into the options. |
| `on-change` | message (index) | — | |
| `role` | [role](#roles) | `primary` | |
| `size` | integer | `16` | |

Children: `option "Label"` — one per choice, in order.

#### `select`

The message a `select` carries is its request to be **opened**: the popup is a
scene the application pushes, since a widget cannot own other nodes. So a select
without `on-change` is a closed control showing what is selected — display-only,
and the honest meaning of an inert one.

| | Type | Default | |
|---|---|---|---|
| `selected` | integer | — | Index; omitted, nothing is selected and the placeholder shows. |
| `placeholder` | string | `""` | |
| `on-change` | message (none) | — | Omitted, the list cannot be opened and the control shows what is chosen. `Select` emits one message and the application reads `selected()` — the exception to the payload table, because a dropdown's selection outlives the event. |
| `role` | [role](#roles) | `base-100` | The control's own surface. |
| `size` | integer | `16` | |

Children: `option "Label"`.

#### `slider`

| | Type | Default | |
|---|---|---|---|
| `min` `max` | number | — | **Required.** |
| `value` | number | `min` | The widget's constructor demands one; `min` is the *format's* default for a slider written without it. |
| `step` | number | — | Continuous without it. |
| `on-change` | message (number) | — | |
| `role` | [role](#roles) | `primary` | |

#### `rating`

| | Type | Default | |
|---|---|---|---|
| `value` | number | `0` | |
| `max` | integer | `5` | How many symbols. |
| `on-change` | message (number) | — | Omitted, the rating is display-only. |
| `clearable` | bool | `#false` | Pressing the current value clears it to zero. |
| `role` | [role](#roles) | `warning` | |

#### `tabs`

| | Type | Default | |
|---|---|---|---|
| `selected` | integer | `0` | |
| `on-change` | message (index) | — | |
| `role` | [role](#roles) | `primary` | The selected tab's underline, and only that. |
| `size` | integer | `16` | |

Children: `tab "Label"`, and each one may carry **its own page**:

```kdl
tabs name=sections x=8 y=8 w=404 h=180 selected=1 on-change=pick {
    tab "First" {
        label "on the first page" x=8 y=8 w=200 h=20
    }
    tab "Second" {
        label "on the second page" x=8 y=8 w=200 h=20
    }
    // Legal, and what a half-written form looks like.
    tab "Third"
}
```

A `tab` with a block nests that section's subtree, and the `tabs` node hosts it:
the strip is drawn in a band along the top and the page fills what is left, so a
widget written at `y=0` inside a tab sits just under the strip. That is the same
arrangement `collapse` uses for its body, and the strip's height is the theme's
field height — [`Tabs::strip_height`], which the builder reads rather than
guessing at.

A `tab` **without** a block is the strip a `tabs` node has always been: the
widget is a tab *bar*, what each tab shows is the application's, and it does that
by showing and hiding panels of its own. Nothing about those forms changed, and
the strip still fills its node — a `tabs h=40` is 40 tall, because making the
band unconditional would have quietly shrunk every strip taller than the theme's
field height.

`selected` is the tab the **application** opens on, counting every `tab` from
zero whether or not it carries a page. Which tab a *designer* is looking at is
the designer's own state and is never written to the file, the way the outline's
eye is never written: selecting a widget on another tab brings that page up.

Only the selected page is in the tree at all. A hidden page paints nothing,
answers no press and takes no caret — so a `tabs` whose pages are all in the
file still costs a panel only the one it shows.

[`Tabs::strip_height`]: https://docs.rs/denise-ui/latest/denise_ui/widgets/struct.Tabs.html#method.strip_height

### Display

#### `progress`

| | Type | Default | |
|---|---|---|---|
| `value` | number | `0` | `0.0`–`1.0`. |
| `role` | [role](#roles) | `primary` | |

#### `radial-progress`

| | Type | Default | |
|---|---|---|---|
| `value` | number | `0` | `0.0`–`1.0`. |
| `label` | string | — | Drawn in the middle. |
| `thickness` | integer | derived | Ring width, derived from the node's size without it. |
| `role` | [role](#roles) | `primary` | |
| `size` | integer | `16` | |

#### `spinner`

| | Type | Default | |
|---|---|---|---|
| `role` | [role](#roles) | `primary` | |
| `thickness` | integer | derived | Derived from the node's size without it. |
| `period-ms` | integer | `1000` | One full turn. Floored at the default sampling interval: a revolution shorter than a frame looks stopped, or looks like it is going backwards. |
| `frame-ms` | integer | — | This spinner's *own* sampling interval, overriding the tree's `Motion`. Almost no form should set it — the rate belongs to the whole panel — and reduced motion overrides it back. |

#### `image`

| | Type | Default | |
|---|---|---|---|
| `src` | [asset](#assets) | — | **Required.** |
| `fit` | [fit](#other-enums) | `contain` | |
| `radius` | integer | `0` | Corner radius in pixels — a number here, not a [radius](#other-enums) token, because a picture is cropped to a shape rather than themed into one. |

#### `video`

| | Type | Default | |
|---|---|---|---|
| `ground` | colour | black | `"#RRGGBB"`. |

The one literal colour in the format. A `video` node is a *hole*: the decoder puts
frames on a hardware plane and the toolkit draws the ground behind it. That colour
is never composited with themed content and has no semantic role to name, so a
role here would be a fiction.

#### `avatar`

| | Type | Default | |
|---|---|---|---|
| `src` | [asset](#assets) | — | A picture. Without one, the initials are drawn. |
| `initials` | string | — | A name; the widget takes its initials. |
| `role` | [role](#roles) | derived | The disc behind the initials — derived *from the initials* without it, so a column of avatars is not one colour. |
| `radius` | integer | half the side | Corner radius; `0` is a square. The side is the smaller of `w` and `h`. |
| `ring` | [role](#roles) | — | A ring around the disc. |
| `presence` | [presence](#other-enums) | — | The status dot. |
| `size` | integer | `16` | |

#### `list`

| | Type | Default | |
|---|---|---|---|
| `selected` | integer | — | |
| `on-select` | message (index) | — | |
| `on-activate` | message (index) | — | Enter, or a double press. |
| `activate-on-click` | bool | `#false` | A single press activates as well as selects. |
| `row-height` | integer | theme's `size_field` | |
| `role` | [role](#roles) | `primary` | The selection. |
| `size` | integer | `16` | |

Children: `item "Text" leading="›" trailing="⌘K" enabled=#false` — `leading` and
`trailing` are short strings at either end of the row; `enabled=#false` is a row
that draws dimmed and cannot be selected.

#### `tree`

| | Type | Default | |
|---|---|---|---|
| `selected` | integer | — | The row's position in the file, counting every `item` whether it is currently drawn or not. |
| `on-select` | message (index) | — | |
| `on-activate` | message (index) | — | Enter, or a double press. |
| `on-toggle` | message (index) | — | A branch opened or shut. |
| `activate-on-click` | bool | `#false` | A single press activates as well as selects. |
| `row-height` | integer | theme's `size_field` | |
| `indent` | integer | `14` | How far one level is pushed in from the one above. |
| `role` | [role](#roles) | `primary` | The selection. |
| `size` | integer | `16` | |

Children: `item "Text" depth=1 open=#false leading="›" trailing="4 °C" enabled=#false`.

**`depth` is the whole hierarchy.** The items are a flat list and each says how
deep it sits; a row's parent is the nearest row above it with a smaller depth,
and its children are the run of deeper rows immediately below. `depth` defaults
to `0`, so a file that says nothing is a flat list.

Nothing says whether a row *has* children, because in this representation that is
not an independent fact — a row has children exactly when the next row is deeper.
A property for it could disagree with the depths, and then one of the two would be
a lie.

`open` defaults to `#true`: a tree that arrived entirely shut would show one row
per branch and nothing of what it is for. `open` on a row with no children is
ignored rather than an error, because a row gains and loses children as the rows
around it change and neither is the file's mistake.

```kdl
tree name=places x=8 y=8 w=224 h=80 selected=1 on-select=go-to-place indent=12 {
    item "Sensors"
    item "Inlet" depth=1 trailing="4 °C"
    item "Outlet" depth=1 trailing="9 °C"
    item "Archive" open=#false
    item "2024" depth=1
}
```

Four rows are drawn: `Archive` is shut, so `2024` is not. It is still row 4 for
`selected` and for every message — a position that changes when a branch above it
folds would be unusable.

#### `table`

| | Type | Default | |
|---|---|---|---|
| `selected` | integer | — | |
| `on-select` | message (index) | — | |
| `on-activate` | message (index) | — | |
| `activate-on-click` | bool | `#false` | |
| `row-height` | integer | theme's `size_field` | |
| `role` | [role](#roles) | `primary` | |
| `size` | integer | `16` | |

Children, in this order:

- `column "Title" width=120 align=end` — `width` in pixels, or `flex=#true` for
  the column that takes what is left. `align` is `start`, `center` or `end`.
- `row "Ada" "Lovelace" "1815"` — one positional argument per column.

#### `timeline`

| | Type | Default | |
|---|---|---|---|
| `row-height` | integer | theme's `size_field` | |
| `size` | integer | `16` | |

Children: `event "Text" time="09:14" role=success pending=#true` — an event's
`role` defaults to `primary`, and `pending` marks one not yet reached, drawn
hollow.

#### `carousel`

| | Type | Default | |
|---|---|---|---|
| `selected` | integer | `0` | The page showing when the form opens. |
| `on-change` | message (index) | — | Emitted with the page a person lands on. The advance clock is silent: a message reports what somebody did. |
| `auto-advance-ms` | integer | — | Advance on the animation clock. Floored at twice the slide duration — an interval the slide cannot keep up with is a carousel that never comes to rest. |
| `role` | [role](#roles) | `primary` | The page dots. |

Children: `picture src="one.png" fit=cover`.

#### `collapse`

A container that folds. Children are its content, placed relative to the node and
**below its header** — so a collapse's child starts at a `y` past the header, and
is clipped while the collapse is closed.

`h` is the node's height in the state `open` declares: the header alone when
closed, the header plus `expanded-height` when open. The header's height is a
theme metric, so a hand-written closed collapse is guessing at it; the engine
honours `h` as written and the toggle animates to `expanded-height` from there.

**Who drives the fold depends on `on-toggle`.** With one, the application is
told and answers with `widgets::set_open` — which is what lets an accordion
close the section beside it, or a fold be refused. Without one, nobody else is
going to, so the section folds itself. A decorative section on a panel therefore
needs no message at all.

| | Type | Default | |
|---|---|---|---|
| *(first argument)* | string | `""` | The header title. |
| `open` | bool | `#true` | |
| `expanded-height` | integer | — | The content's height when open. Measured from the children without it. |
| `on-toggle` | message (bool) | — | Omitted, the section folds itself and reports nothing. |
| `role` | [role](#roles) | `base-200` | The header. |
| `size` | integer | `16` | |

---

## Hand-editing

The reason all of the above is worth the trouble. Nudging a button down by eight
pixels, in an editor:

```diff
-        button "Greet" x=20 y=128 w=110 h=34 role=primary on-press=greet size=16
+        button "Greet" x=20 y=136 w=110 h=34 role=primary on-press=greet size=16
```

The same nudge on the canvas, in the designer, produces the same diff. That is the
whole promise, and it is enforced rather than hoped for.

What survives a designer save:

- Comments — including one on the line of a property that was then changed, and
  one written above a node that was then deleted and undeleted.
- Blank lines and indentation the author chose, including columns lined up by
  hand and an indent that is not the designer's.
- The order properties were written in. A property added goes on the end; one
  reset to its default is taken out rather than spelled out.
- A property written twice. KDL says the last one wins and this crate agrees, but
  the file still says both after a save: somebody edited that line and did not
  finish, and the fix is theirs to make rather than a tool's to make silently.
- A file with no trailing newline, which stays that way.

What does not survive: nothing. If the designer changes a byte the user did not
ask it to change, that is a bug with a test waiting for it.

### How it is checked

[`denise-forms/tests/awkward/`](../denise-forms/tests/awkward/README.md) is a
corpus of forms written the way a person writes them — comments in every position
KDL allows one, hand-aligned columns, a property written three times, strings with
escapes and emoji in them, a panel with no braces, no trailing newline. Two tests
walk it, so defending a new way of writing a form by hand is adding a file and
nothing else:

- `denise-forms/tests/awkward.rs` — parse to text, the format's own round trip,
  and one targeted edit each.
- `tools/designer/src/app.rs` — `Document::open` to `Document::save`, through a
  real file, which is the path a person's form actually takes.

Both assert that a save with no edits changes no byte, and that moving one node
changes exactly the line that node is written on — the same line, with the number
changed, every other property still in the order the file wrote them and whatever
comment was on the end still on the end.

A corpus only defends what somebody thought of, so the fuzz target `parse_form`
([`fuzz/README.md`](../fuzz/README.md)) throws bytes at `Form::parse` and asserts
the round trip on whatever comes back — which is how the two shapes kdl was
quietly eating got found in the first place, and how the corpus gained
`after-the-brace.dform`. `Form::parse` also verifies each file for itself, so a
shape nobody has fuzzed yet is refused rather than corrupted.

An **unknown property is an error at load**, not something kept and shown as
unknown. That is the deliberate choice described [above](#the-document) and it is
the same error a typo produces, with the same fix; refusing to open is louder than
dropping the property and loses nothing.

And it is a two-way street while both are open: the designer watches the file it
has open and reads it again when something else writes it, so an editor and a
canvas can be pointed at the same form at the same time. See [the designer's
README](../tools/designer/README.md#the-other-editor).

## Loading one from Rust

Five lines, and the whole of what an application does with a form:

```rust
use denise_forms::{Form, Handler, Payload};
use denise_ui::Ui;

let form = Form::parse(include_str!("../forms/hello.dform"))?;

// The form says how big it is and which theme it wants; nothing here says it
// twice.
let mut ui: Ui<Message> = Ui::new(form.size(), form.theme());
let root = ui.root();

let built = form.build(&mut ui, root, &mut |name: &str, payload: Payload| {
    match (name, payload) {
        ("greet", Payload::None) => Some(Handler::Plain(Message::Greet)),
        _ => None,
    }
})?;

// The nodes the file named, by the names it used.
let field = built.node("who");
```

[`examples/designed`](../examples/designed) is that, complete and runnable — the
`hello` example again, from [`hello.dform`](../forms/hello.dform) rather than from
twenty lines of `ui.add`. Read the two side by side: **a form replaces the
tree-building and nothing else.** The message enum, the `update`, the damage and
the event loop are the same file in both.

### The three things a file cannot hold

A `.dform` holds widgets and their initial state. It holds no code, so three
things stay the application's, and the closure above (or a `Wiring` impl, where
there are pictures too) is where it says them.

**Its own message type.** The file says `on-press=greet`; only the application
knows that `greet` is `Message::Greet`. The mapping is a match on a string, which
the compiler cannot check — so a name the form uses and the application does not
answer is an error *at load*, with the name in it, rather than a button that
quietly does nothing.

`payload` is which **shape** the widget needs, and it is why this is a `Handler`
and not a closure returning `M`. A button holds a message; a checkbox holds a
`fn(bool) -> M`, which is a *function pointer* that no closure built from a name
could ever be. An enum's tuple variant already is one — `Handler::Bool(Message::Notify)`
— which is the whole trick.

| `payload` | The widget wants | Answer with |
|---|---|---|
| `Payload::None` | the message itself | `Handler::Plain(Message::Save)` |
| `Payload::Bool` | `fn(bool) -> M` | `Handler::Bool(Message::Notify)` |
| `Payload::Index` | `fn(usize) -> M` | `Handler::Index(Message::Chose)` |
| `Payload::Number` | `fn(f32) -> M` | `Handler::Number(Message::Level)` |

**Its pictures.** `image src="logo.png"` is a path relative to the form file, and
`Wiring::asset` is what turns it into pixels. This crate decodes nothing and does
not depend on `denise-image`, which keeps a board with its pictures compiled in
from linking a decoder it will never call.

**What the widgets are called.** `name=who` in the file becomes
`built.node("who")`, and that is the one place a typo shows up as a `None` rather
than as a compile error — unless the names are generated, which is
[the next section](#or-let-the-compiler-check-it).

### Or let the compiler check it

Everything above is strings checked at load. A build script turns the same file
into a struct and an enum instead:

```rust
// build.rs
fn main() {
    denise_forms::codegen::to_out_dir("forms/hello.dform").unwrap();
}
```

```rust
include!(concat!(env!("OUT_DIR"), "/hello.rs"));

let form = Hello::build(&mut ui, root)?;      // no `Wiring` to write
let field = form.who;                          // a field, not a lookup

match message {
    HelloMessage::Greet => self.greet(),       // exhaustive, and checked
}
```

**Rename a node in the designer and the application stops compiling**, naming the
field that is gone. **Add a message to the form and every `match` stops being
exhaustive.** Neither is something a string lookup can do, and both are asserted
with `trybuild` rather than described.

Or skip the `match`. The same file generates a trait with one method per
message, named for it and taking what it carries, and a `dispatch` that calls
the method a message is named for:

```rust
impl HelloHandlers for App {
    fn greet(&mut self) { … }
}

for message in ui.drain_messages() {
    message.dispatch(&mut app);
}
```

Now **adding an event in the designer makes the compiler name the method that is
missing** — `greet`, at `impl HelloHandlers for App` — which is also the method
the designer writes when asked to open that event. The binding from form to
code is one `impl` line, in code, where a type name means something; the form
file still says nothing about code.

The generated `build` calls the same engine — there is one implementation, and
this is a typed door onto it — so a form loaded at runtime and the same form
generated behave identically. `Hello::place` is the door onto `build_fitted`, so
a generated form still scales by its own [`scaling=`](#scaling).

A **build script rather than a proc macro**, deliberately: the output is a file
you can open, `cargo doc` sees it, a debugger steps through it, and it needs no
second crate.

Three things it refuses rather than generating something wrong: a name Rust
cannot spell (`name="2"`), two names that become one field (`full-name` and
`full_name`), and one message name used with two payload shapes — the engine is
happy with the last, because a `match` answers each call separately, and an enum
variant cannot be both a value and a `fn(bool) -> M`.

[`examples/typed`](../examples/typed) is `hello` a third time, this way. All
three draw the same pixels, which is checked rather than claimed.

### Baked in, or read at runtime

`include_str!` compiles the form into the binary: a kiosk image with no writable
filesystem still gets its layout, and the file is checked at build time by being a
string literal and at load time by `Form::parse`. `std::fs::read_to_string` is
the other way, and is what you want when the form is meant to be swapped without
a rebuild — a panel whose screens are updated by copying files.

[`examples/runtime`](../examples/runtime) is the other way, with two screens:
each read from its file when it is shown, one `Wiring` answering both, and the
tests that say what that path can and cannot promise.

Both are the same three lines afterwards. The format does not care — with one
exception, which is that a form read at runtime may have come from anywhere.
`Form::parse_within` is `Form::parse` with a deadline, and [it is what to call
when the file is not yours](#and-a-fourth-which-is-a-clock).

The **typed** path above is the one place this is a real choice rather than a
preference: generating the struct bakes the form in at build time, because that
is what makes the names checkable. A panel whose screens are updated by copying
files wants the untyped path, and it is not a lesser one.

## Checking a file

```bash
cargo install denise-forms --features cli

denise-forms check settings.dform            # exit 1, with positions
denise-forms render settings.dform out.ppm   # --theme light, --font path.ttf
denise-forms fmt   settings.dform            # --check to report and write nothing
```

`check` parses the file, **builds it into a real widget tree** — the same code a
panel runs — and reports what is wrong as `file:line:column: message`. CI runs it
over every `.dform` in this repository, so a form added broken does not stay
broken.

`render` draws one frame into a PPM with no display attached, which is how a
layout is reviewed over SSH and how a theme change is diffed before and after. It
is deterministic: the same file renders the same bytes twice, because without
`--font` it uses the built-in bitmap font rather than whatever the machine
happens to have installed.

`check` also **lints geometry**, which nothing else can: with no layout engine,
a node quietly sitting outside its parent or on top of its sibling is legal and
usually a mistake. Both are warnings rather than errors, because both are
sometimes meant — a scrim covers the surface on purpose, a `collapse`'s content
sits outside it while closed, and a stack or a viewport places its children
somewhere other than where the rectangles say. `--no-lint` turns it off. Writing
[`reference.dform`](../forms/reference.dform) by hand produced two of each within
an hour, which is the argument for it.

`fmt` lays a file's indentation out again and changes nothing else. One step of
indent per level of nesting, trailing whitespace gone, and **only the whitespace
at the two ends of a line is ever touched** — so comments keep their text and
their position, strings keep their quoting, properties keep their order, blank
lines stay blank lines, and columns lined up by hand inside a line stay lined
up. The step is the file's own, whatever the first node inside `form` uses, so a
file written with two spaces stays a two-space file. `--check` writes nothing and
exits non-zero if anything would change, which is what CI runs.

It is deliberately not a canonical formatter, and that is the story of
[#87](https://github.com/bisand/denise/issues/87) and
[#119](https://github.com/bisand/denise/issues/119). A canonical one was written
first, on `kdl`'s own `autoformat`, and not shipped: it **deletes a comment
written at the end of a node's line**
([kdl-org/kdl-rs#179](https://github.com/kdl-org/kdl-rs/issues/179)), and it
unquotes strings and drops blank lines besides. A tool in this repository that
silently eats a comment is worse than no tool — somebody runs it over a form they
annotated, the annotations go, and the loss is buried in a diff of a hundred
reformatted lines. Re-indenting is the thing hand-editing actually breaks, and it
is the thing that can be done without touching a byte anybody wrote.

Two properties are tested over every `.dform` in the repository rather than by
example, because the point of the tool is that it can be run without reading the
diff: every line of the output is a line of the input with its ends trimmed, and
laying out a laid-out file changes nothing.

### Three limits, all of them about the parser

`MAX_SOURCE` (4 MB), `MAX_DEPTH` (64) and `MAX_COMMENTED_DEPTH` (1), plus a
check that the braces balance at all, are applied by a byte scan **before** the
file reaches `kdl`, and none of them is about what a form is allowed to mean. Each one is a place where the parser costs more than
a `Result` can express: an enormous file costs memory, nesting past 64 overflows
a recursive descent, and — found by fuzzing (#104) — a commented-out block
nested inside another commented-out block sends `kdl` exponential, at about a
hundred bytes for twenty seconds. Unbalanced braces are the bluntest of them:
such a file cannot parse however long `kdl` spends deciding that, and the
fuzzer's slow inputs are unbalanced to a one. A guard a caller cannot install
for itself belongs in the crate that knows why it is there.

That last one bounds a shape, not the parser. `kdl` 6.7.1 has other exponential
corners, and agreeing with it about where a string ends means *being* its lexer
— a fourth divergence turned up five minutes after the third was fixed. Anything
that slips past the scan costs whatever `kdl` costs, which is the next section.

### And a fourth, which is a clock

`Form::parse_within(source, limit)` is `Form::parse` with a deadline. It is the
complete answer to the exponential corners, because the only thing that bounds a
parser you cannot change is giving up on it, and it is what anything reading a
form **it did not write** should call — opened by a person, pasted, downloaded,
handed over on a stick, or watched on disk while a text editor has it too.

`denise_forms::PATIENCE` is one second, which is the default because it is three
hundred times the slowest real form: [`reference.dform`](../forms/reference.dform),
every node kind this toolkit has in nine and a half kilobytes, parses in **under
3 ms** on an M5 Pro, and the other five forms in the repository in under 300 µs
each. The limit is an argument rather than a constant because two things move it:
a file near `MAX_SOURCE` is legitimately slower — four megabytes of real nodes
measures 1.7 s — and a slower machine is slower.

The honest part is what happens when the deadline passes. **A parse is abandoned,
not stopped.** A thread cannot be cancelled and `kdl` has no point at which to ask
it to stop, so the call returns `Reason::TooSlow` and the worker keeps going until
it finishes, which for the exponential shapes may be never. That bounds the call
and not the process, so `MAX_ABANDONED` (4) bounds the process: a parse whose
predecessors are still wedged is refused before it starts, with `Reason::NoThread`
and a message saying to restart. Four wedged threads on a four-core panel is the
point at which spawning a fifth stops being caution and starts being the denial of
service the deadline is there to prevent.

### Who bounds it

| caller | bounded | why |
|---|---|---|
| **The designer** | ✅ `PATIENCE` | On open, on a paste, and on an outside write noticed by the file watcher. It opens whatever it is pointed at, and it is the case with a person sitting in front of it. |
| **An application reading a form at runtime** | your call | `Form::parse_within` is there; `Form::parse` is still the short one. Bound it if the file can come from anywhere but your own repository. |
| **An application with the form baked in** | not at risk | `include_str!` reads a file in the repository at build time, and CI has already parsed it. |
| **The `denise-forms` command** | ❌ deliberately | A command with a terminal in front of it and a person who can stop it, run on files you chose. A limit here would refuse a legitimately enormous generated form and buy nothing a `^C` does not. |

## What the format will not do

Every one of these is a decision, not an omission.

**No layout engine.** There are no constraints and no percentages. Nodes are
rectangles relative to their parent, because that is what a fixed-resolution panel
wants and it is what the toolkit does; the file cannot express what the toolkit
cannot draw. `hello`'s card is centred by arithmetic in `main.rs` and
[`hello.dform`](../forms/hello.dform) puts it at a fixed offset instead — the one
visible difference between them, and an honest one. The designer's answer to
placement is snapping and alignment
([#92](https://github.com/bisand/denise/issues/92),
[#95](https://github.com/bisand/denise/issues/95)), which is a tool for a person
rather than a solver at runtime. What a form *can* do about being shown at a size
it was not designed at is [below](#responsiveness).

**No expressions, no bindings, no scripting.** No `visible={not loading}`. A form
file describes a tree of widgets and their initial state; everything that
*happens* is Rust, in the one `match` the toolkit is built around. A format that
grows a little language grows a debugger, a scope model and a security boundary,
and the browser example is in this repository partly to show what declining that
buys.

**No colours.** Roles, and the theme decides. The single exception is
[`video`](#video)'s ground, and it is explained where it is.

**No per-widget styling.** No fonts per node in v1, no padding, no margins, no
shadows. Depth and radius are theme tokens because the toolkit made them theme
tokens, and a form that could override them is a form that stops matching the
rest of the application.

**One form per file.** A file that held several would need names for them and a
way to say which one is meant, and the filesystem already has both.

## Responsiveness

A form file in version 1 describes one rectangle per node at one size. That is
the honest state of it, and worth saying outright rather than leaving a reader to
discover it: **a form designed at 1024×600 is a form that works at 1024×600.**

The reason is not the format. It is that `denise-ui` positions every node by an
explicit rectangle and the parent never tells a child it resized, so a file that
said anything else would be saying something nothing could render. The fix
therefore belongs in the toolkit first and the schema second, and it is three
separate problems that are easy to run together:

| | | |
|---|---|---|
| **Scale** | Same design, more pixels. A 1024×600 form on a 2048×1200 panel, or on a 2× display. | **Done** — [`scaling=`](#scaling) below ([#111](https://github.com/bisand/denise/issues/111)): the engine multiplies on the way in, exactly as `hello` does by hand, and the form declares whether it consents to being scaled at all. |
| **Resize** | A window being dragged, a different aspect ratio, a panel turned to portrait. Scaling alone letterboxes or distorts. | **Done** — `anchor=` and `dock=` per node ([#110](https://github.com/bisand/denise/issues/110)): Delphi's and WinForms' own answer, one derived rectangle per child in the reflow pass the tree already runs, and [#86](https://github.com/bisand/denise/issues/86) builds them from the file. |
| **Content-driven sizing** | A label as wide as its text, a row that grows with what is in it. | [#112](https://github.com/bisand/denise/issues/112) — a real measure-and-arrange engine, in a crate of its own that an application opts into, so the core keeps costing nothing. **Designed, not built**: [docs/arrange.md](arrange.md). A form file will not describe it — a file that meant different things depending on the reader's dependency graph is the worst property a format can have. |

Both of the first two are **properties added to existing nodes**, which
`version 1` allows without a bump — nesting, naming, roles, messages and
collections are untouched, and every form written before them keeps working
because their defaults are today's behaviour.

What none of them is: a phone-style reflow from three columns to one. That needs
per-size layout overrides, which KDL can carry later as `at width<=800 { … }`
child nodes with no version bump. It is deliberately not being built yet — for two
genuinely different layouts, two form files is a good answer, and no panel has
asked for the other thing.

### Scaling

```kdl
form "Dashboard" version=1 kind=screen width=1024 height=600 scaling=proportional
```

| `scaling` | |
|---|---|
| `none` | Never scaled. Drawn at its design size, centred in whatever it is given. **The default**, because it is what every form written before this property existed already did. |
| `proportional` | One factor on both axes — `min(target.w / design.w, target.h / design.h)` — so nothing distorts. The leftover is a margin on the axis that had room to spare. |
| `stretch` | A factor per axis, filling the surface. Distorts, and is occasionally exactly what a signage layout wants. |

**The form decides, not the application.** Scaling is not always right and the
form is the thing that knows: a dial designed against a 1:1 photographic
background, a layout whose text must stay a legal minimum size, a panel whose
touch targets are already the smallest a gloved finger can hit. Each of those is
a form that should be shown at its design size and centred, whatever the panel it
lands on. So it is declared in the file rather than decided by whoever loaded it.

Loading one is three lines, and **all three matter**:

```rust
let fit = form.fit(surface);                                    // what the file's rule works out to
let mut ui = Ui::new(surface, form.theme().scaled(fit.uniform()));   // or the widgets are the old size
let stage = ui.add(root, Panel::filled(form.background()), fit.rect);
let built = form.build_fitted(&mut ui, stage, fit, &mut wiring)?;
```

The theme is the easy one to forget, and forgetting it is visible: a button at 2×
with a 6-pixel corner and a 1-pixel border on it. `Ui` has one theme, so scaling
it is the caller's line rather than something `build_fitted` could do.

[`examples/designed`](../examples/designed) is exactly those lines. Change
`hello.dform`'s form node to say `scaling=proportional` and it fills the window,
with no Rust changing.

**What scales, and what does not.** Every rectangle, and every number a widget
declares to be a length in logical pixels — a text size, a row height, a border
width, a ring thickness. Not a duration, not a count, not a selected index. The
widget is what says which of its own numbers is which, through
`Property::in_pixels`, for the same reason the rest of the descriptor exists:
there is no table of widgets anywhere in this repository, and this is not the
place to start one.

Rectangles scale **by their edges** (`Rect::scaled_by`), so two panels designed to
touch still touch at 0.75×. Scaling width and height instead rounds each
independently and opens one-pixel seams. Lengths that are not rectangles use the
smaller of the two axis factors, so a stretched layout never grows text taller
than the axis with least room to give; and a length that would round to nothing
keeps one pixel, because deleting a hairline is a visible change while `0` in the
file was somebody saying *none*.

**Text scaling is a DPI answer, not a "bigger screen" answer.** A 1024×600 form
on a 1920×1080 panel gets its 16 px text at 30 px. That is right when the panel is
the same screen at a higher density, and wrong when it is a bigger screen meant to
show *more*. This does the first. The second is not a multiplication and no file
can express it — it is two form files, or [#112](https://github.com/bisand/denise/issues/112).

**Scaling and anchoring compose.** Scale is a *deployment* concern applied once on
the way in; [`anchor=`](#every-node) is a *design* concern the tree resolves at
every reflow. A form may use either, both or neither.

You can see it without a display:

```bash
denise-forms render --scale 2    forms/reference.dform reference-2x.ppm
denise-forms render --scale 0.75 forms/reference.dform reference-075x.ppm
denise-forms render --size 1920x1080 forms/reference.dform panel.ppm
```

`--scale` is "the same panel at a higher density" and the picture grows with the
form. `--size` is "this actual panel": the surface is what you asked for and the
file's own `scaling=` decides what happens inside it, which is the part that
cannot be reviewed any other way. The reference form at each, committed:
[1×](../assets/screenshots/reference-1x.png),
[2×](../assets/screenshots/reference-2x.png),
[0.75×](../assets/screenshots/reference-075x.png).

## What is still open

Decided elsewhere, and deliberately not settled here:

- A `tabs` node whose pages are laid out by the file rather than nested under
  each `tab` — a form that wants two tabs sharing one page, say. Nesting answers
  the common case and a shared page has no spelling yet.
- A named registry of built-in icons, which `button icon=` needs before it can
  exist.
- A font-name resolver, once a form in this repository needs two fonts.
- Custom themes: `theme=` takes a built-in name today. A theme exported from the
  gallery's editor is nine seed colours and a name, which is a small file of its
  own rather than something to inline here.

One thing this document changed upstream of itself, now settled: the engine's
resolver was written in [#86](https://github.com/bisand/denise/issues/86) as
`Fn(&str) -> Option<M>`, and the [payload table](#messages) showed that could not
build a checkbox or a slider. It turned out not to be enough even with the
payload — those widgets hold a `fn(bool) -> M`, a **function pointer**, which no
closure built from a name can be. `denise-forms` asks for a `Handler<M>` instead,
and an application answers with its own enum's tuple variant, which already is
one.
