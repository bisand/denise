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
parsed byte for byte. Requirement 3 is then met by the library rather than by our
discipline, which is the only way it stays met.

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

Tab order is file order in v1. [#98](https://github.com/bisand/denise/issues/98)
decides whether that stays true or gains a `tab-index`.

### Geometry

Four integers, not one packed string. `x=20 y=44 w=388 h=34` costs a few
characters over `rect="20 44 388 34"` and buys real KDL numbers: an error points
at the number rather than at the string containing it, an editor treats them as
numbers, and nothing has to parse inside a value. Delphi wrote `Left`, `Top`,
`Width`, `Height` on four lines and nobody ever complained about it.

Negative values are allowed and mean what they say — a node partly outside its
parent, clipped by it. That is the rendered truth and the designer draws it.

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

Whether a given collection is real data or design-time placeholder is a live
question — a `select`'s options usually are the real ones, a `table`'s rows never
are — and [#105](https://github.com/bisand/denise/issues/105) decides it. In
version 1 they are all simply built; an application that means to replace them
calls `set_rows` afterwards, exactly as `table-editor` does today.

---

## The widgets

Twenty-five widgets and one container. Every property is optional unless marked
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

`on-change` is **required**: `Select` holds a message of the application's type
and has no inert constructor, so there is nothing a form file could put there
instead. Same for [`collapse`](#collapse). Both are worth a `::inert` in
`denise-ui` and do not have one yet.

| | Type | Default | |
|---|---|---|---|
| `selected` | integer | — | Index; omitted, nothing is selected and the placeholder shows. |
| `placeholder` | string | `""` | |
| `on-change` | message (none) | — | `Select` emits one message and the application reads `selected()` — the exception to the payload table, because a dropdown's selection outlives the event. |
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

Children: `tab "Label"` — labels only. The widget is a tab *bar*; what each tab
shows is the application's, which today means showing and hiding sibling panels.
Whether the file should nest a subtree per tab is
[#105](https://github.com/bisand/denise/issues/105).

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

| | Type | Default | |
|---|---|---|---|
| *(first argument)* | string | `""` | The header title. |
| `open` | bool | `#true` | |
| `expanded-height` | integer | — | The content's height when open. Measured from the children without it. |
| `on-toggle` | message (bool) | — | |
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
than as a compile error. [#101](https://github.com/bisand/denise/issues/101) is
where the names get generated instead.

### Baked in, or read at runtime

`include_str!` compiles the form into the binary: a kiosk image with no writable
filesystem still gets its layout, and the file is checked at build time by being a
string literal and at load time by `Form::parse`. `std::fs::read_to_string` is
the other way, and is what you want when the form is meant to be swapped without
a rebuild — a panel whose screens are updated by copying files.

Both are the same three lines afterwards. The format does not care.

## Checking a file

```bash
cargo install denise-forms --features cli

denise-forms check settings.dform            # exit 1, with positions
denise-forms render settings.dform out.ppm   # --theme light, --font path.ttf
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

There is no `fmt`. There was going to be
([#87](https://github.com/bisand/denise/issues/87) asked for one), and `kdl`'s
own formatter turns out to delete a comment written at the end of a node's line —
which is not a thing to ship into a format whose first promise is that comments
survive. [#119](https://github.com/bisand/denise/issues/119) is where that sits.

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
| **Content-driven sizing** | A label as wide as its text, a row that grows with what is in it. | [#112](https://github.com/bisand/denise/issues/112) — a real measure-and-arrange engine, in a crate of its own that an application opts into, so the core keeps costing nothing. |

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

- Which collections are design-time placeholder data and which are real, and what
  a `tabs` node's content looks like — [#105](https://github.com/bisand/denise/issues/105).
- Whether tab order stays "file order" or gains a `tab-index` —
  [#98](https://github.com/bisand/denise/issues/98).
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
