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

## What this crate does not do

**It does not open anything.** `Form::kind` reports that a file is a dialog;
whether that becomes `Ui::push_scene` on a panel or a modal window on a desktop is
the application's decision, and it differs by machine.

**It does not lay anything out.** Nodes are rectangles, with the toolkit's own
anchors and docking over them. There is no solver here and none below.
