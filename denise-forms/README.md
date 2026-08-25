# denise-forms

Loads a [DeniseUI](https://github.com/bisand/denise) form file into a widget tree
at runtime.

```toml
[dependencies]
denise-forms = "0.19"
```

A form in Denise is ordinarily Rust: a `Ui<M>`, a tree of `ui.add(parent, widget,
rect)` calls, and a `match` over the messages that come back. This crate is the
other way to say the same thing — a `.dform` file a visual designer writes, a
person edits, `git diff` reads, and this loads.

The file format is [documented in full](https://github.com/bisand/denise/blob/main/docs/forms.md):
KDL, one form per file, one rectangle per node.

## Building a form

```rust
# use denise::{Rect, Size, theme};
# use denise_ui::{Ui, Void};
use denise_forms::{Form, Handler, Payload};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Message {
    Greet,
}

let form = Form::parse(r#"
    form "Hello" version=1 kind=screen width=460 height=260 {
        label "What is your name?" x=20 y=20 w=388 h=20
        text-input name=who x=20 y=44 w=388 h=34
        button "Greet" x=20 y=90 w=110 h=34 role=primary on-press=greet
    }
"#)?;

let mut ui: Ui<Message> = Ui::new(form.size(), form.theme());
let root = ui.root();

let built = form.build(&mut ui, root, &mut |name: &str, _: Payload| match name {
    "greet" => Some(Handler::Plain(Message::Greet)),
    _ => None,
})?;

// Nodes the file named can be found again; the rest need no name.
let field = built.node("who").expect("the field");
# assert!(ui.contains(field));
# Ok::<(), denise_forms::Error>(())
```

## Messages are names, and the application owns them

A widget holds a value of *your* message type and this crate has never seen your
type, so the file holds a **name** and you map it. That closure is the whole
bridge, and an unknown name is an error at load with the name in it — not a button
that silently does nothing, which is the failure every string-keyed UI format has
and the reason people distrust them.

The `Handler` you return is not always a plain message, because the widgets are
not all the same shape. A `Button` holds an `M`; a `Checkbox` holds a
`fn(bool) -> M`; a `List` holds a `fn(usize) -> M`; a `Slider` holds a
`fn(f32) -> M`. Those are **function pointers**, so no closure of this crate's can
stand in for one — which is exactly what an enum's tuple variant already is:

```rust
# use denise_forms::{Handler, Payload};
#[derive(Clone, Copy)]
enum Message {
    Save,
    Notify(bool),
    Pick(usize),
}

let resolve = |name: &str, _: Payload| match name {
    "save" => Some(Handler::Plain(Message::Save)),
    // `Message::Notify` *is* a `fn(bool) -> Message`.
    "set-notify" => Some(Handler::Bool(Message::Notify)),
    "pick" => Some(Handler::Index(Message::Pick)),
    _ => None,
};
# let _ = resolve("save", Payload::None);
```

The `Payload` you are handed says which shape the widget wants, so a resolver
that serves several can answer without guessing. Returning the wrong one is an
error naming the message and what was needed.

## Pictures

`src` on an `image`, an `avatar` or a `carousel`'s pictures is a path **relative
to the form file**, never to the working directory — so a form and its pictures
move together and a kiosk's current directory is not part of the contract.

This crate decodes nothing and does not depend on `denise-image`: implement
[`Wiring::asset`] and hand back pixels. That is also how a board with its pictures
compiled in serves them from a table instead of touching a disk.

## Errors are the product

A form is something a person typed, so every failure carries a line, a column, and
— where there is a finite set of right answers — the whole set:

```text
14:9: `button` has no property `colour`; it accepts corner, no-focus, on-press,
      radius, repeat-delay, repeat-interval, role, size, text, watch-hold
```

## The command-line tool

```bash
cargo install denise-forms --features cli
```

Behind a feature, and not a default one: the library's job on a panel is to turn
a file into widgets, and a kiosk linking it should not also link three image
decoders because a command-line tool needs them to draw a picture into a PPM.

```bash
denise-forms check settings.dform            # exit 1, with positions
denise-forms check $(git ls-files '*.dform')
denise-forms render settings.dform out.ppm   # --theme light, --font path.ttf
```

`check` parses the file **and builds it**, so it catches everything a panel
would, and prints `file:line:column: message`. It also lints geometry — a node
outside its parent, a pair of siblings on top of each other — as warnings, since
with no layout engine nothing else ever will. `--no-lint` turns that off,
`--quiet` says nothing unless something is wrong.

`render` draws one frame with no display attached. Deterministic: without
`--font` it uses the built-in bitmap font rather than whatever the machine has
installed, so two renders of one file are the same bytes and a snapshot is worth
committing. `--font` needs the `truetype` feature.

## Editing one, byte for byte

The other half of the crate, and the one the [designer](https://github.com/bisand/denise/tree/main/tools/designer)
is built on. A `Form` holds the **document**, not a struct taken from it, so an
edit changes what it names and nothing else:

```rust
# use denise_forms::{Edit, Form};
// A comment, and columns somebody lined up by hand.
let source = "\
// The panel everything sits on.
form \"F\" version=1 width=320 height=240 {
    label \"One\"   x=8  y=8  w=80 h=20
    label \"Two\"   x=8  y=32 w=80 h=20
}
";
let mut form = Form::parse(source)?;

let undo = form.apply(Edit::number(&[1], "y", Some(40)))?;
assert_eq!(form.text(), source.replace("x=8  y=32", "x=8  y=40"));

// Every edit hands back the edit that reverses it, so undo is applying that —
// there is no snapshot of anything, anywhere.
form.apply(undo)?;
assert_eq!(form.text(), source);
# Ok::<(), denise_forms::Error>(())
```

The inverse carries the **text** that was there and not the value, which is why
this is byte-exact rather than merely correct: `1_000`, `0x10` and `70.0` are
values a typed inverse would carry, and none of them would be written back the
way they were written down.

That this holds for files people actually write, rather than only for the ones in
the doc comments, is what [`tests/awkward/`](https://github.com/bisand/denise/tree/main/denise-forms/tests/awkward)
is for: a corpus with comments in every position KDL allows one, columns lined up
by hand, a property written three times, strings with escapes and emoji in them,
a panel with no braces and a file with no trailing newline. The tests walk the
directory, so defending a new way of writing a form by hand is adding a file.

`Edit::Move` reparents a node — its children, its indentation and the comment
above it all going with it — and `Edit::Many` makes a run of edits one step, so a
gesture that changes four numbers is one thing to undo. The empty path is the
`form` node itself, so its size, kind and theme are edited through the same door.

[`Form::node_text`] hands one node back as source and [`fragment`] reads source
back into nodes, which is how copy and paste carry `.dform` text on the system
clipboard: paste into a text editor and you have the source, paste from one and
you have the nodes.

[`Form::node_text`]: https://docs.rs/denise-forms/latest/denise_forms/struct.Form.html#method.node_text
[`fragment`]: https://docs.rs/denise-forms/latest/denise_forms/fn.fragment.html

## What this crate does not do

**It does not open anything.** `Form::kind` reports that a file is a dialog;
whether that becomes `Ui::push_scene` on a panel or a modal window on a desktop is
the application's decision, and it differs by machine.

**It does not lay anything out.** Nodes are rectangles, with the toolkit's own
anchors and docking over them. There is no solver here and none below.

**It does not know what a clipboard or a window is.** Editing a form is text in,
text out; whose clipboard that text came from is the tool's business, which is
why `arboard` is a dependency of the designer and never of this crate.
