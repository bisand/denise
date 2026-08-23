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

![The designer with a slider selected and being dragged: the palette and outline on the left, the reference form on the canvas with a selection outline, eight handles and a name tag, an alignment guide running down the panel, and the slider's own properties in the inspector](../../assets/screenshots/designer.png)

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

## What is here

| | |
|---|---|
| **Palette** | Every widget the toolkit ships, from `widgets::all()`. This file names none of them, so a twenty-sixth appears without it changing. |
| **Outline** | The nodes the open form named, and picking one selects it. |
| **Canvas** | The form, drawn — and selected, moved, resized and deleted. |
| **Inspector** | The selected node's properties, from the widget's own `Describe` — again, no list here. |
| **Toolbar** | New, Open, Save, Save as. |

Open a form, save it, and `git diff` is empty: the document is what is held, not a
value taken from it, so a save that changed nothing changes nothing. That is the
round trip the whole file format is built around, and there is a test for it.

## What is not here yet

Dragging from the palette is [#91] and editing a property is [#93] — the
inspector reports, it does not yet edit.

Three parts of [#92] are not done either: a rubber band over empty space, dropping
a node onto a panel to reparent it, and bring-to-front and send-to-back. Each is
its own mechanism rather than a corner of what is here.

[#91]: https://github.com/bisand/denise/issues/91
[#92]: https://github.com/bisand/denise/issues/92
[#93]: https://github.com/bisand/denise/issues/93

## Elsewhere

`--snapshot out.ppm` draws one frame and exits, with no window — the same
affordance every example in this repository has, and how this one's own layout
gets reviewed over SSH or diffed in a pull request.

The window size and the pane widths are remembered in the platform's own
configuration directory, as four `key = value` lines.
