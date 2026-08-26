# From a drawing to a running screen

How the designer, the file and an application fit together, end to end.

This is the *workflow* document. What the designer's panes are and what every
gesture does is [its own README](../tools/designer/README.md); what the file
format is and why is [docs/forms.md](forms.md). This is the path between them,
which neither of those is about.

The short version: **there is no build step and no export.** The designer writes
the file, the application reads the file, and the file is text. Nothing is
generated, so nothing can be out of date.

```
   designer  ──writes──▶  settings.dform  ◀──reads──  your application
      ▲                        │
      └────── your editor ─────┘
```

## 1. Get the designer

Every [release](https://github.com/bisand/denise/releases/latest) carries a build
for macOS, Windows and Linux — no Rust toolchain needed. It is unsigned, so the
first launch takes one extra click; [the designer's
README](../tools/designer/README.md#getting-it-without-a-rust-toolchain) says
which.

From a clone, it is three words:

```bash
cargo run -p denise-designer
```

## 2. Draw the form

*New* asks what kind of form it is — a screen, a window, a dialog, a drawer, a
shelf or a fragment — because the kind decides what the rest of the questions
are, and because a form's kind is a fact the file records rather than something
the application asserts later.

Then: drag widgets out of the palette, drag them about, drag their handles,
align them, and give the ones the application will need to reach a **name**.

Two things are worth knowing before the first drag:

**There is no layout engine, on purpose.** Every node is a rectangle relative to
its parent. That is what a fixed-resolution panel wants, it is what the toolkit
does, and the file cannot express what the toolkit cannot draw. The designer's
answer to placement is snapping, alignment guides and the align/distribute
commands — tools for a person rather than a solver at runtime.

**The canvas is not a preview.** It draws the form with *the same code that will
draw it on the panel* — the same widgets, the same rasteriser, the same theme
roles — so what is on screen is what ships, to the pixel. `F5` goes further and
lets the form actually run: the scrim over it hides, the events become the
form's, and a strip along the bottom names every message it fires.

## 3. Save it, and read it

```kdl
form "Settings" name=settings version=1 kind=screen width=1024 height=600 theme=dark {

    panel name=card x=32 y=32 w=960 h=536 {
        label "Settings" x=24 y=20 w=400 h=28 size=20

        text-input name=who x=24 y=72 w=400 h=34 placeholder="your name" on-submit=save

        button "Save" x=24 y=124 w=110 h=34 role=primary on-press=save
    }
}
```

That is the whole file. It is in `git diff`, it reviews like code, and the next
person can nudge a button eight pixels down in an editor without opening the
designer at all — which is the point of the format and is
[tested](forms.md#hand-editing) rather than hoped for.

**Both can be open at once.** The designer writes through a temporary file and a
rename, so the editor never reads a half-written form; and it reads the file again
when the editor saves, within about half a second, keeping the selection by name.
Nudge the rectangle in Vim and watch it move on the canvas. If there is unsaved
work in the designer when that happens it asks — naming the nodes that differ —
rather than one side quietly winning. [The designer's
README](../tools/designer/README.md#the-other-editor) has the picture.

Before wiring it up, ask the CLI what it thinks:

```bash
denise-forms check settings.dform
```

It parses the file, **builds it into a real widget tree** — the same code a panel
runs — and reports what is wrong as `file:line:column: message`. It also lints
geometry, which nothing else can: with no layout engine, a node quietly sitting
outside its parent is legal and usually a mistake.

## 4. Wire it into an application

```rust
let form = Form::parse(include_str!("settings.dform"))?;
let mut ui: Ui<Message> = Ui::new(form.size(), form.theme());
let root = ui.root();

let built = form.build(&mut ui, root, &mut |name: &str, payload: Payload| {
    match (name, payload) {
        ("save", Payload::None) => Some(Handler::Plain(Message::Save)),
        _ => None,
    }
})?;

let who = built.node("who").expect("the form names a field `who`");
```

The three things the file cannot hold — the application's message type, its
pictures, and what the widgets are called — are
[explained in full](forms.md#loading-one-from-rust) with the payload table.

[`examples/designed`](../examples/designed) is a complete, runnable version of
exactly this. It is the [`hello`](../examples/hello) example again, built from
[`hello.dform`](../forms/hello.dform), and reading the two side by side is the
fastest way to see what a form does and does not change:

```bash
cargo run -p hello        # the tree, in Rust
cargo run -p designed     # the same tree, from the file
```

They draw the same pixels, and not as a figure of speech:

```bash
cargo run -p hello    -- --snapshot a.ppm
cargo run -p designed -- --snapshot b.ppm
cmp a.ppm b.ppm       # silent
```

The message enum, the `update`, the damage handling and the event loop are the
same code in both. **A form replaces the tree-building and nothing else.**

And a third way, when the form is baked in rather than swapped: a four-line
`build.rs` turns it into a struct whose fields are the form's names and an enum
whose variants are its messages, so renaming a node in the designer stops the
application compiling instead of returning `None`.
[`examples/typed`](../examples/typed) is that, and
[docs/forms.md](forms.md#or-let-the-compiler-check-it) is the argument.

## 5. Ship it

Which display the application talks to is decided at compile time, and the form
has no opinion about it:

```bash
cargo run -p designed                                   # a window, for development
cargo build -p designed --no-default-features \
    --features kiosk --release \
    --target aarch64-unknown-linux-musl                 # a panel with no desktop
```

The form file goes along either way — compiled in with `include_str!` for an
image with no writable filesystem, or read at runtime when the screens are meant
to be updated by copying files.

## Where this does not go

The [README's known gaps](../README.md#known-gaps-deliberately-not-hidden) apply
to forms as much as to the toolkit, and two of them are worth repeating here
because they are the ones that surprise people arriving from a web stack:

**No layout engine.** Covered above, and the one place the two `hello`s really
do differ: [`examples/hello`](../examples/hello) centres its card with
arithmetic against the surface size, and
[`hello.dform`](../forms/hello.dform) places it. At the size the form was
designed at those come to the same pixel, which is why the two snapshots match
byte for byte; on a surface of some other size the arithmetic follows and the
rectangle does not. That is exactly the boundary, and
[docs/forms.md](forms.md#what-the-format-will-not-do) is where it is argued.

What to do about it is the **file's** decision, not the application's, because
the file is what knows whether the design tolerates it. `hello.dform` says
nothing, which means `scaling=none`, which means its own rectangle centred in
whatever surface it is given: a 460x260 form on a 1920x1080 panel is a 460x260
form in the middle. Change that one word to `proportional` and `designed` fills
the panel, with no Rust changing — the three lines that load a form already ask.
See [Scaling](forms.md#scaling).

**No expressions, no bindings, no scripting.** There is no `visible={not
loading}`. A form file describes a tree of widgets and their initial state;
everything that *happens* is Rust, in the one `match` the toolkit is built
around. A format that grows a little language grows a debugger, a scope model and
a security boundary with it.

Neither is a gap waiting to be filled. Both are what make a form file something
you can read in ten seconds and diff in one.

## The rest of it

| | |
|---|---|
| [The designer's README](../tools/designer/README.md) | The panes, every gesture, every key, and why the canvas is not a second `Ui`. |
| [docs/forms.md](forms.md) | The format: why a file at all, why KDL, the whole v1 schema, and what it will never do. |
| [`denise-forms`](../denise-forms) | The crate an application loads a form with, and the CLI that checks one. |
| [`forms/`](../forms) | The reference form — every node kind and every property, once — and a small file for each kind of form. |
