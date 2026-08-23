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

## What is here

| | |
|---|---|
| **Palette** | Every widget the toolkit ships, from `widgets::all()`. This file names none of them, so a twenty-sixth appears without it changing. |
| **Outline** | The nodes the open form named, and picking one selects it. |
| **Canvas** | The form, drawn. |
| **Inspector** | The selected node's properties, from the widget's own `Describe` — again, no list here. |
| **Toolbar** | New, Open, Save, Save as. |

Open a form, save it, and `git diff` is empty: the document is what is held, not a
value taken from it, so a save that changed nothing changes nothing. That is the
round trip the whole file format is built around, and there is a test for it.

## What is not here yet

Dragging from the palette is [#91], moving and resizing on the canvas is [#92],
editing a property is [#93], and undo is [#94]. The inspector reports; it does not
yet edit.

[#91]: https://github.com/bisand/denise/issues/91
[#92]: https://github.com/bisand/denise/issues/92
[#93]: https://github.com/bisand/denise/issues/93
[#94]: https://github.com/bisand/denise/issues/94

## Elsewhere

`--snapshot out.ppm` draws one frame and exits, with no window — the same
affordance every example in this repository has, and how this one's own layout
gets reviewed over SSH or diffed in a pull request.

The window size and the pane widths are remembered in the platform's own
configuration directory, as four `key = value` lines.
