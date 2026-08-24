# denise-designer

A visual form designer for [DeniseUI](https://github.com/bisand/denise): the thing
Delphi had in 1995 and the WinForms designer kept.

```bash
cargo run -p denise-designer -- forms/reference.dform
```

It reads and writes [`.dform` files](../../docs/forms.md), which are text, and
which [`denise-forms`](../../denise-forms) loads on the panel.

## It is a Denise application

Not Tauri, not egui, not a web page. The canvas draws the form with **the same
code that will draw it on the panel** — the same widgets, the same rasteriser,
the same theme roles — so what is on screen is what ships, to the pixel, rather
than an approximation somebody has to keep in step.

That also makes it the first real application written on this toolkit, which is
worth more than the convenience: what the toolkit lacks turns up here first. It
already has. The pane layout is `Dock`, and building it found a case where a
reflow starting *at* a docked node computed its rectangle from its own layout
instead of from its siblings — a whole pane with the right gap beside it and
nothing in it.

## The canvas is not a second `Ui`

The form is built into a subtree of the designer's own tree, inside a scrolling
viewport. One tree, one event loop, one damage tracker.

What keeps the form from *behaving* while it is being designed is a **scrim**: an
invisible `Panel` with `backdrop` set, sitting over the form and above it. It
absorbs every press and leaves the focus exactly where it was — the same
affordance that lets an on-screen keyboard type into a field without the field
losing its caret. Preview mode is hiding the scrim, and that is the whole of
preview mode.

![The designer with a slider selected and being dragged: the palette and outline on the left, the reference form on the canvas with a selection outline, eight handles and a name tag, an alignment guide running down the panel, and on the right the slider's own properties — a field each for the rectangle, boxes for `visible` and `enabled`, a dropdown for `role`, and the ones the file does not write dimmed](../../assets/screenshots/designer.png)

## Design mode

A press on the canvas never reaches the form. Design mode reads the events first
and does **its own hit test**, because the tree's is about what a *running* form
should react to and that is not the same question: a `Label` answers `false` to
`accepts_pointer`, so the tree would send a click straight past it — right on a
panel, where a label sitting on a button must not swallow the press, and wrong
here, where clicking a label has to select the label. A node that is not *drawn*
is skipped, so an invisible sheet over the whole form does not eat every click.

| | |
|---|---|
| Click | Selects what is under it. Shift adds and takes away. Escape clears. |
| Tab | Walks the form in file order; Shift+Tab back. |
| Drag the body | Moves. Drag a handle: resizes. |
| Arrow keys | One pixel. With Shift, twice the grid. |
| Delete | Takes the node and everything under it out of the file. |
| G | Turns snapping off, and on again. |
| Ctrl/Cmd-Z | Undo. With Shift: redo. |
| Enter | Puts down whatever the palette has armed. Escape gives it up. |
| F2 | Renames the selected node, in the outline, in place. |

Snapping is to a 4-pixel grid and, in preference to it, to the edges and centres
of the node's **siblings** — the only alignment that means anything when there is
no layout engine. A guide is drawn on whatever it lined up with.

**A drag is one edit.** The tree moves while the pointer does, so it can be seen;
the file is written once, on release, as a targeted edit to that node's line. A
press that never moved writes nothing at all, because it was a selection. That is
what keeps a move to a one-line diff, and what makes it one undo step.

## Undo

There is no snapshot of anything. `Form::apply` hands back the edit that reverses
the one just made, so undo is applying that, and what *it* hands back is the redo.
The whole mechanism is two stacks of edits and a marker for where the file was
last saved — which is only possible because the thing being edited is the
document, comments and spacing included, rather than a value taken from it.

So an undo is exact. Delete a panel and undo it, and the panel comes back with
its children, its indentation, **and the comment written above it** — that
comment is part of the node, and removing the node took it too.

A drag is one step even when it moved and resized at once, and a run of nudges to
one property is one step until you work on something else. The title carries a `•`
while the file on disk is behind, and undoing back to the last save takes the mark
away again.

Closing with unsaved work stops once and says so; ask again and it goes. A modal
would be the better question and needs a second window — this is the honest
version until then, and it is at least impossible to lose a form to one keystroke.

## The palette

Every widget the toolkit ships, from `widgets::all()` — this crate names none of
them, so a twenty-sixth appears without it changing. The field above filters.

Two ways to put one on the form, and they share their machinery:

- **Drag** a row onto the canvas. A ghost follows the pointer the whole way,
  across from one pane to the other, and the widget lands where it is let go of
  at whatever size it usually starts at.
- **Click** a row, then **drag a rectangle** on the canvas, the WinForms way — or
  press **Enter** and one goes down without drawing anything, stepping clear of
  whatever is already there so the second does not hide under the first.

![A button being dragged out of the palette: the palette on the left with a filter above it, and a ghost outline labelled `button` following the pointer over the form on the canvas](../../assets/screenshots/designer-place.png)

Either way it is **parented to whatever container it was dropped in** — which is a
`panel` or a `collapse`, because those are the two that lay children out. A
`select` holds options and a `table` holds columns; dropping a button on one of
those has missed, and it goes on the form instead. The rectangle written to the
file is relative to the parent, which is the only space a form file knows.

What lands is the least the file can say: a rectangle, and for five widgets one
thing more, because an `alert` has no colour to draw itself in without a `role`
and a `slider` has no range without `min` and `max`. That comes from
`denise_forms::seed`, which lives next to the code that raises those
requirements so the two cannot drift.

The new node is selected the moment it lands, so the inspector is already
describing it, and the whole placement is one undo.

## The outline

Every node of the form, indented, in file order — not only the ones with names.
That is what the pane is *for*: the canvas cannot show a node behind another, one
clipped out of its parent, one sized to nothing, or the ninetieth identical row of
a table, and all of those have to be reachable.

```
- panel header          o
    label               o
    badge               o
    spinner busy        .
- panel sidebar         o
    list nav            o
```

The kind, then the name. A `-` or `+` folds a subtree. The mark on the right is an
**eye**: press it and the node is hidden *in the designer only* — `x` — which is
how you reach what is sitting behind it. Nothing about that is written to the
file, and the form is not marked as modified. A node the **file** hides, with
`visible=#false`, shows a `.` instead: the eye did not do that and cannot undo it,
and the inspector's `visible` row is where it is changed.

Selection is shared with the canvas both ways: click a row and the canvas draws
handles round it; click the canvas and the row highlights.

**Drag a row** to reorder it among its siblings or to reparent it — dropping on
the middle of a `panel` or a `collapse` puts it inside, dropping on an edge puts
it beside. A marker shows which. That is one `denise_forms::Edit::Move`, so it is
one undo step, and the node is **re-indented** for its new depth, children and
all, because a file whose nesting and whose indentation disagree is one somebody
has to fix by hand.

**F2** renames the selected node in place. Enter keeps it, Escape does not.

`-` and `+` and `o` rather than triangles and an eye glyph because the built-in
5×7 font covers ASCII and Latin-1 and nothing else, and a `▾` draws the
missing-character box. Which is what every tree control drew before it had the
glyphs for anything better.

## The inspector

Select something and its properties fill the right pane, each with an editor
chosen from its type: a field for a string, a box for a flag, a dropdown for a
role, a field with a slider beside it for a number over a range you can aim at.

**Nothing in the designer lists them.** A row exists because the widget's own
`Describe` implementation says the property exists, and the properties the *tree*
owns — `x`, `visible`, `dock` — come from `denise_forms::NODE_PROPERTIES`,
described the same way. A twenty-sixth widget, or a twenty-seventh property on an
existing one, gets its editors without a line of this crate changing.

Edits apply **as they are typed**, through the same `set` the engine calls when it
loads a form — so the pane cannot show a value the engine could not load, and one
the widget refuses is reported in the error role rather than written. An `x` of
`over there` never reaches the file.

A property the file does not write is at its default, and is dimmed to say so. The
`×` beside one it does write takes it back out, which is what returning to a
default means when the schema does not spell defaults out. Clearing a field does
the same thing.

Where the file already writes a property as the node's **argument** — `label
"Heading"`, which is how every form here is written — that is what an edit
changes. Adding `text="…"` beside it would leave the file saying one thing and the
screen showing another.

Select several and the pane shows what they have in common, blank where they
disagree, and an edit goes to all of them as a single undo step.

Two things the pane writes but the canvas cannot immediately show: a message name,
which is a value of the *application's* type and so is not something any widget
can be handed, and a property cleared back to a default that only exists once the
widget is built again. Both are written to the file at once and the canvas catches
up when the caret leaves the field — rebuilding mid-keystroke would take the caret
out of the field being typed in.

## What is here

| | |
|---|---|
| **Palette** | Every widget the toolkit ships, from `widgets::all()`, filtered by the field above — dragged or clicked onto the canvas. |
| **Outline** | Every node, as a tree: folded, selected, renamed, dragged to reparent, and hidden here without the file knowing. |
| **Canvas** | The form, drawn — and selected, moved, resized and deleted. |
| **Inspector** | The selected node's properties, edited — from the widget's own `Describe`, so again no list here. |
| **Toolbar** | New, Open, Save, Save as, Undo, Redo. |

Open a form, save it, and `git diff` is empty: the document is what is held, not a
value taken from it, so a save that changed nothing changes nothing. That is the
round trip the whole file format is built around, and there is a test for it.

## What is not here yet

The palette is a flat list. Grouping it, and giving each row an icon and a
tooltip saying what the widget *is*, wants somewhere for that to come from: the
registry carries a name and a property list and nothing else, and a table of
descriptions in this crate is exactly what the registry exists to avoid. That
wants a line of documentation on each widget's `Describe`, which is [#126].

Three parts of [#92] are not done: a rubber band over empty space, dropping a node
onto a panel **on the canvas** to reparent it, and bring-to-front and send-to-back.
The reparent edit itself is here — the outline drives it — so the canvas drag is
the gesture and not the mechanism.

The outline remembers what is folded and what is hidden **by path**, so an edit
that shifts a path leaves those pointing at whatever moved into its place. A form
is small and a click puts it right; the alternative is giving every node an
identity the file does not have.

The **form's own** properties — its title, its size, its theme — are not in the
pane: it shows the selected *node*, and the form node cannot be selected. Its
size is what the canvas is, so changing it wants somewhere of its own.

A message field is a field, not a combo box: the name being typed is usually one
the form has not used yet, and this toolkit has no widget that is both. What the
form *does* already use is in the row's tooltip.

[#92]: https://github.com/bisand/denise/issues/92
[#126]: https://github.com/bisand/denise/issues/126

## Elsewhere

`--snapshot out.ppm` draws one frame and exits, with no window — with `--select`,
`--drag` and `--carry` to pose it, since a snapshot has no pointer to select,
drag or carry anything with — the same
affordance every example in this repository has, and how this one's own layout
gets reviewed over SSH or diffed in a pull request.

The window size and the pane widths are remembered in the platform's own
configuration directory, as four `key = value` lines.
