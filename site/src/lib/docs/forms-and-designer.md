---
title: Forms and the designer
description: The .dform file, loading one with denise-forms, checking it from the command line, and drawing one in the visual designer.
---

A screen in DeniseUI is ordinarily Rust: a tree of `ui.add` calls and a `match` over the messages that come back. A `.dform` file is the other way to say the same thing — text that a visual designer writes, a person edits, `git diff` reads, and `denise-forms` loads. **There is no build step and no export**, so nothing is generated that could go out of date.

## The file

A `.dform` is [KDL](https://kdl.dev) version 2, one form per file, and this is the version 1 schema. KDL was chosen over RON, TOML, JSON, YAML and XML for one reason above the others: the `kdl` crate is document-oriented, so comments, spacing and property order survive a round trip. Open a form in the designer, save it, and `git diff` is empty.

This is [`forms/hello.dform`](https://github.com/bisand/denise/blob/main/forms/hello.dform), the `hello` example as a file:

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

### The form node

| Property | |
|---|---|
| title (first argument) | Required. |
| `version` | Required, `1`. An engine refuses a version it does not know, in a sentence. |
| `kind` | `screen` (default), `window`, `dialog`, `drawer`, `shelf` or `fragment`. The engine reports the kind; the application decides what opens it. |
| `width`, `height` | Required. The size the form was designed at. |
| `theme` | `dark` (default), `light` or `high-contrast`. |
| `scaling` | `none` (default), `proportional` or `stretch`. |

### Every node

A node's name is its widget kind — `label`, `button`, `text-input` — with an optional positional argument for its primary text. `x`, `y`, `w` and `h` are **required** and relative to the parent. Beyond those, any node may carry `name`, `visible`, `enabled`, `z`, `tooltip`, `scroll`, `stack`, `focus`, `anchor` and `dock`. A property at its default is not written.

A few rules settle most questions:

- **Tab order is file order**, depth first. There is no `tab-index`; to change the order, move the node in the file.
- **Roles, not colours.** A form says `role=primary`. The single literal colour in the format is a `video` node's ground, because a video plane has no themed role to name.
- **Messages are names.** `on-press=greet` names a message; the application says what `greet` means. An unknown name is an error at load, naming the name.
- **Placeholder content stays out of the build.** A table's sample rows go in a `design` block, which only the designer reads, so they never reach a panel.
- **An unknown property is an error**, the same error a typo produces.

What a form file will not do, on purpose: no layout engine, no expressions or bindings, no colours, no per-widget fonts, padding or shadows, and no more than one form per file.

## Loading a form

```rust
use denise_forms::{Form, Handler, Payload};
use denise_ui::Ui;

let form = Form::parse(include_str!("../forms/hello.dform"))?;

// The form says how big it is and which theme it wants.
let mut ui: Ui<Message> = Ui::new(form.size(), form.theme());
let root = ui.root();

let built = form.build(&mut ui, root, &mut |name: &str, payload: Payload| {
    match (name, payload) {
        ("greet", Payload::None) => Some(Handler::Plain(Message::Greet)),
        _ => None,
    }
})?;

let field = built.node("who");
```

A widget holds a value of the application's type, and some widgets hold a *function* rather than a value. `Payload` says which shape the widget wants, and an enum's tuple variant already is the function it needs:

| `payload` | The widget holds | Answer with |
|---|---|---|
| `Payload::None` | the message | `Handler::Plain(Message::Save)` |
| `Payload::Bool` | `fn(bool) -> M` | `Handler::Bool(Message::Notify)` |
| `Payload::Index` | `fn(usize) -> M` | `Handler::Index(Message::Chose)` |
| `Payload::Number` | `fn(f32) -> M` | `Handler::Number(Message::Level)` |

Pictures are the application's too: `src` is a path relative to the form file, and `Wiring::asset` turns it into pixels. `denise-forms` decodes nothing.

[`examples/designed`](https://github.com/bisand/denise/tree/main/examples/designed) is `hello` built from that file. The message enum, the update, the damage handling and the event loop are the same code as `examples/hello`, and the two draw the same bytes:

```bash
cargo run -p hello    -- --snapshot a.ppm
cargo run -p designed -- --snapshot b.ppm
cmp a.ppm b.ppm       # silent
```

### Baked in or read at run time

`include_str!` compiles the form into the binary, which suits an image with no writable filesystem. `std::fs::read_to_string` suits a panel whose screens are updated by copying files — [`examples/runtime`](https://github.com/bisand/denise/tree/main/examples/runtime) reads two forms that way. A form read at run time may have come from anywhere, so parse it with `Form::parse_within` and a deadline: `kdl` parses some malformed documents in exponential time, and `denise_forms::PATIENCE` is one second.

### Or let the compiler check the names

With the `codegen` feature, a build script turns the form into a struct with a field per named node and an enum with a variant per message:

```rust
// build.rs
fn main() {
    denise_forms::codegen::to_out_dir("forms/hello.dform").unwrap();
}
```

Rename a node in the designer and the application stops compiling, naming the field that went. [`examples/typed`](https://github.com/bisand/denise/tree/main/examples/typed) is `hello` a third time, this way.

### Drawing at another size

A form designed at 1024×600 is a form that works at 1024×600 unless it says otherwise. `scaling=proportional` or `scaling=stretch` consents to being scaled, and loading it is `form.fit(surface)`, a theme scaled by `fit.uniform()`, and `form.build_fitted`. Rectangles scale by their edges, so panels designed to touch still touch. Per-node `anchor` and `dock` handle a window being resized. Content-driven sizing is not something a form file describes.

## The command-line tool

```bash
cargo install denise-forms --features cli

denise-forms check settings.dform              # exit 1, with positions
denise-forms render settings.dform out.ppm     # --theme light, --font path.ttf
denise-forms render --scale 2 settings.dform out.ppm
denise-forms render --size 1920x1080 settings.dform panel.ppm
denise-forms fmt settings.dform                # --check to report and write nothing
```

`check` parses the file and **builds it into a real widget tree**, so it catches what a panel would, and reports `file:line:column: message`. It also warns about geometry nothing else can see without a layout engine — a node outside its parent, or siblings on top of each other; `--no-lint` turns that off. `render` is deterministic, using the built-in bitmap font unless given `--font`. `fmt` re-indents and touches only the whitespace at the ends of lines, so comments, quoting and hand-aligned columns survive. The release downloads include this tool beside the designer.

## The designer

```bash
cargo run -p denise-designer -- forms/reference.dform
```

Or [download it](https://github.com/bisand/denise/releases/latest), with no Rust toolchain: a `.dmg` for macOS, universal for Intel and Apple silicon; a `.zip` for Windows; and a `.tar.gz` each for x86-64 and aarch64 Linux, with a `.sha256` beside each.

**The builds are not signed.** There is no Apple Developer account or Windows code-signing certificate behind the project. On macOS, right-click *Denise Designer.app*, choose **Open**, then **Open** again; macOS remembers. On Windows, SmartScreen offers **More info**, then **Run anyway**. On Linux, `tar xzf` and run it.

### It is a Denise application

Not Tauri, not egui, not a web page. The canvas draws the form with the same widgets, rasteriser and theme roles that will draw it on the panel, so what is on screen is what ships, to the pixel. It holds no list of widgets: the palette is `widgets::all()` and the inspector draws one editor per described property, so a new widget turns up in both without the designer changing.

### Working on a form

Drag widgets from the palette and drag them about or by their handles, snapping to a 4-pixel grid and to siblings' edges and centres. Select with a click or a rubber band, then align, size, space and group from the arrange bar. Arrow keys nudge a pixel; PageUp and PageDown reorder a node among its siblings by moving it in the file; Ctrl or Cmd with C, X and V carry `.dform` source on the system clipboard; F2 renames. **Tab order** mode numbers the stops and lets you click them into a new order.

**F5 runs the form.** The invisible sheet that absorbs presses in design mode is hidden, the events become the form's, and a strip along the bottom names every message it fires — so `on-press=greet` can be checked before the application exists. Escape or F5 goes back, rebuilding from the file.

### Undo is exact

The designer does not keep a struct and serialise it on save; **its model is the document**. Every canvas edit is a targeted edit to the text, and `Form::apply` hands back the edit that reverses it. Undo applies that, byte for byte. Delete a panel and undo, and it comes back with its children, its indentation and the comment written above it. A drag is one edit, written once on release, which is why moving a button is a one-line diff and one undo step.

### Your editor is the other half

The designer saves through a temporary file and a rename, so an editor never reads a half-written form, and it reads the file again within about half a second when something else writes it, keeping the selection **by name**. With unsaved work it asks instead, listing the nodes that differ and offering **Reload** or **Keep mine**. It compares contents rather than timestamps, because on Windows the clock ticks about every 16 ms and a second write in the same tick would go unseen.

Beside every event in the inspector is a button that opens its handler in your editor, writing a placeholder if nothing answers the name yet. Which source file that is gets remembered in a sidecar beside the form, such as `hello.designer`.

### Not there yet

A message field is a plain field rather than a combo box, and the outline remembers folded and hidden nodes by path, so an edit that shifts a path leaves those pointing at whatever moved into its place.

Further reading: [docs/forms.md](https://github.com/bisand/denise/blob/main/docs/forms.md) is the full schema, [docs/designer.md](https://github.com/bisand/denise/blob/main/docs/designer.md) the workflow, and [the designer's README](https://github.com/bisand/denise/blob/main/tools/designer/README.md) every pane and gesture. API: [docs.rs/denise-forms](https://docs.rs/denise-forms).
