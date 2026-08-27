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
denise-forms render --scale 2 settings.dform out.ppm
denise-forms render --size 1920x1080 settings.dform panel.ppm
denise-forms fmt   settings.dform            # --check to report and write nothing
```

`check` parses the file **and builds it**, so it catches everything a panel
would, and prints `file:line:column: message`. It also lints geometry — a node
outside its parent, a pair of siblings on top of each other — as warnings, since
with no layout engine nothing else ever will. `--no-lint` turns that off,
`--quiet` says nothing unless something is wrong.

`fmt` lays the indentation out again and changes nothing else: one step per level
of nesting, trailing whitespace gone, and **only the whitespace at the two ends
of a line is ever touched**. Comments keep their text and their place, strings
keep their quoting, properties keep their order, blank lines stay blank lines,
and columns lined up by hand inside a line stay lined up. The step is the file's
own — whatever the first node inside `form` uses — so a two-space file stays a
two-space file. `--check` writes nothing and exits non-zero if anything would
change.

It is not a canonical formatter on purpose. `kdl`'s own deletes a comment written
at the end of a node's line, which is not a thing to ship into a format whose
first promise is that comments survive; re-indenting is the part hand-editing
actually breaks, and the part that can be done without touching a byte anybody
wrote. See [`tidy`], which is the same thing from Rust.

`render` draws one frame with no display attached. Deterministic: without
`--font` it uses the built-in bitmap font rather than whatever the machine has
installed, so two renders of one file are the same bytes and a snapshot is worth
committing. `--font` draws the whole form in that face instead, and needs the
`truetype` feature.

`--scale` and `--size` ask different questions. `--scale 2` is *the same panel at
twice the density*: one factor, and the picture grows with the form. `--size
1920x1080` is *this actual panel*: the surface is what you asked for, and the
file's own `scaling=` decides what happens inside it — its own size in the middle,
scaled to fit, or stretched. That last one cannot be reviewed any other way.

## Scaling one

A form declares whether it may be drawn at a size other than the one it was
designed at, because the form is the thing that knows: a dial against a 1:1
photographic background, or a panel whose touch targets are already the smallest
a gloved finger can hit, should be centred rather than stretched however big the
display is.

```kdl
form "Dashboard" version=1 kind=screen width=1024 height=600 scaling=proportional
```

`none` (the default, and what every form written before the property existed
already did), `proportional`, or `stretch`. Loading it is three lines, and all
three matter:

```rust
# use denise::Size;
# use denise_forms::Form;
# use denise_ui::{Ui, Void, widgets::Panel};
# let form = Form::parse(r#"form "F" version=1 width=200 height=100 { }"#)?;
# let surface = Size::new(400, 400);
# let mut wiring = |_: &str, _: denise_forms::Payload| None;
let fit = form.fit(surface);

// The theme too, or every widget is the old size inside a new rectangle.
let mut ui: Ui<Void> = Ui::new(surface, form.theme().scaled(fit.uniform()));
let root = ui.root();
let stage = ui.add(root, Panel::filled(form.background()), fit.rect).unwrap();

let built = form.build_fitted(&mut ui, stage, fit, &mut wiring)?;
# Ok::<(), denise_forms::Error>(())
```

Every rectangle scales **by its edges**, so two panels designed to touch still
touch at 0.75×. Every number a widget declares to be a length in logical pixels —
a text size, a row height, a border width — scales with it; a duration, a count
and a selected index do not. The widget is what says which of its own numbers is
which, so there is no table of widgets here.

The full argument, including why text scaling is a DPI answer rather than a
"bigger screen" answer, is in
[docs/forms.md](https://github.com/bisand/denise/blob/main/docs/forms.md#scaling).

## Or let the compiler check the names

```toml
[build-dependencies]
denise-forms = { version = "0.19", features = ["codegen"] }
```

```text
// build.rs
fn main() {
    denise_forms::codegen::to_out_dir("forms/hello.dform").unwrap();
}
```

Out of it comes a `struct Hello` with a `NodeId` field per named node and an
`enum HelloMessage` with a variant per message the form emits, carrying that
widget's payload. Rename a node and the application stops compiling; add a
message and every `match` stops being exhaustive.

A build script rather than a proc macro, deliberately: the output is a file you
can open, `cargo doc` sees it, and it needs no second crate. The generated
`build` calls this same engine, so a form loaded at runtime and the same form
generated behave identically.

Only the build script links the generator — the feature is off by default, and a
panel links the engine and nothing else.

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

A corpus only covers what somebody thought of, so [`Form::parse`] does not take
the round trip on trust: it writes the document back out and compares it to the
source, and a file it cannot reproduce is **refused** ([`Reason::NotPreserved`])
rather than accepted and corrupted on the first save. That check is also what
the fuzz target `parse_form` asserts on, which is how the two shapes kdl was
quietly eating — trailing whitespace after a closing brace, and a comment
written on the brace's line — were found and put back. See
[`fuzz/README.md`](https://github.com/bisand/denise/tree/main/fuzz).

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
[`tidy`]: https://docs.rs/denise-forms/latest/denise_forms/fn.tidy.html
[`Form::parse`]: https://docs.rs/denise-forms/latest/denise_forms/struct.Form.html#method.parse
[`Reason::NotPreserved`]: https://docs.rs/denise-forms/latest/denise_forms/enum.Reason.html#variant.NotPreserved

## What this crate does not do

**It does not open anything.** `Form::kind` reports that a file is a dialog;
whether that becomes `Ui::push_scene` on a panel or a modal window on a desktop is
the application's decision, and it differs by machine.

**It does not lay anything out.** Nodes are rectangles, with the toolkit's own
anchors and docking over them. There is no solver here and none below.

**It does not know what a clipboard or a window is.** Editing a form is text in,
text out; whose clipboard that text came from is the tool's business, which is
why `arboard` is a dependency of the designer and never of this crate.
