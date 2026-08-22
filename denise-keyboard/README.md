# denise-keyboard

An on-screen keyboard for [Denise](https://github.com/bisand/denise) panels
that have no other one — a shelf of keys that slides up from the bottom and
emits exactly what a keyboard plugged into the machine emits.

Nothing downstream can tell the difference. `denise` splits keyboard input into
`InputEvent::Key`, a physical position, and `InputEvent::Text`, a character
somebody meant to insert; the hardware path emits the first followed by
whatever the second turns out to be, and so does this. The application hands
them to `Ui::handle`, which is the call the hardware path's events arrive
through as well — so a `TextInput` inserts them without knowing, a binding on
Enter fires as it would, and widgets that are not text fields hear the keyboard
too.

```rust
use denise::{KeyCode, Rect, Size, theme};
use denise_keyboard::Keyboard;
use denise_ui::{Ui, widgets::TextInput};

#[derive(Clone, Debug, PartialEq)]
enum Msg {
    Key(KeyCode),
}

let mut ui: Ui<Msg> = Ui::new(Size::new(800, 480), theme::DARK);
let root = ui.root();
let field = ui.add(root, TextInput::new(), Rect::new(20, 20, 400, 40)).unwrap();
ui.focus(Some(field));

// Whatever the machine is configured for; US when it says nothing. The scale
// and the style are worth handing it: see "Fitting the panel" below.
let mut keyboard = Keyboard::new(denise_layout::from_system().0);

// Once a frame, beside draining messages: the keyboard arrives when focus
// lands on a text field and leaves when it goes anywhere else.
keyboard.follow_focus(&mut ui, Msg::Key);
assert!(keyboard.is_open());
ui.tick(0);

// What the application's message loop does with a key.
for message in [Msg::Key(KeyCode::H), Msg::Key(KeyCode::I)] {
    match message {
        Msg::Key(code) => {
            let events = keyboard.press(code);
            ui.handle(&events);
        }
    }
}

assert_eq!(ui.widget::<TextInput<Msg>>(field).unwrap().text(), "hi");
// The field never lost the caret, which is the point.
assert_eq!(ui.focused(), Some(field));
```

`follow_focus` is the ordinary policy and nothing more: it asks the tree
whether the newly focused node is a `TextInput` and opens or closes
accordingly. An application wanting a different rule reads
`Ui::focus_changed()` itself and decides — the toolkit reports that focus
moved and takes no view on what it means, because whether a node deserves a
keyboard is not a question `denise-ui` can answer.

Escape is yours to bind. A shelf pushes no scene, so the tree does not claim
the key and will not close the keyboard behind your back.

## Fitting the panel

Two things the application knows and a widget cannot, both builders on
`Keyboard`:

`with_scale` turns the grid's logical pixels into the surface's own. `KEY_HEIGHT`
is 48 because that is a fingertip, not because it is a count of device pixels —
on a 2× panel a keyboard that ignored the scale would draw half-size keys beside
full-size everything else. Same `scale` the application scales its own layout by.

`with_style` names the face the legends are drawn in. Given none, a `Button`
falls back to the built-in 8×8 bitmap face, and the result is visible: the one
widget somebody is touching, drawn in the one typeface that is not the rest of
the application. `set_style` is the same thing for an application that registers
its font after building its tree.

The keys wear roles rather than the toolkit's default: characters are
`Base100`, the modifiers and the layout key are `Neutral`, and Enter is
`Primary`. Forty `Primary` buttons at once is not emphasis.

## How it stays out of the way

A key is a `Button::no_focus()`, so pressing one moves no focus; the keys sit
on a `Ui::push_shelf`, which slides in without pushing a scene, so the field
keeps focus while the keyboard is up. Neither is a keyboard feature — they are
toolkit features this is the first user of, and both are in `denise-ui`.

## Layouts

The tables come from `denise-layout`, the same ones `denise-evdev` feeds from
the hardware, so a dead key composes here exactly as it does there: `¨` then
`o` is one `ö`, and `¨` then `q` is `¨q` rather than a swallowed mark.

Keys are laid out by *position*, not by character. `KeyCode::Semicolon` carries
`ø` on a Norwegian layout and `;` on a US one, and the key does not move —
which is why switching layouts relabels the keyboard rather than rebuilding it.

## Modifiers

Shift cycles — off, once, locked — because there is no clock in the press path
and a double-tap window needs one. The key says which state it is in.

`Locked` is Caps Lock rather than a held Shift: it applies to letters and
spares the digit row, which is the difference between a locked keyboard typing
`1` and typing `!`.

The third level is the layout's own `AltGr`, not a page of symbols chosen here
— `@` is `AltGr`+`2` on Norwegian and `Shift`+`2` on US, so a fixed grid would
be wrong on one of them.

## Layouts, and switching them

`Keyboard::from_system()` starts from whatever the machine is configured for —
the same answer the hardware path starts from — and hands back a
`LayoutSource` saying where it came from. `LayoutSource::Unknown` means the
system asked for a layout there is no table for and got US, which is worth
putting in front of somebody rather than leaving them to wonder.

The layout key walks the built-ins: `us`, `no`, `de`. Switching **reletters the
keys where they stand**, because a position does not move when the layout
changes. German is the layout that proves it — QWERTZ, so `KeyCode::Y` types
`z`, and a keyboard lettered from key *names* would be wrong on two rows.

Switching this keyboard does not switch a physical one attached to the same
machine; call `InputBackend::set_layout` too if you want them in step.

## Seeing what you are typing

Focusing a field scrolls it clear of the keyboard, not merely into its
viewport — the tree knows a shelf is in the way. A viewport with nothing to
scroll cannot do that, and for those `Keyboard::occluded()` says exactly what
the application has to move something clear of.

What to *do* about it is the application's, because only it knows what may move,
and the two demos in this repository need different answers:

- The **browser** grows its page by `Keyboard::height()` while the keyboard is
  up, so a field in the last screenful has somewhere to scroll into. Every phone
  browser does this.
- The **table editor** cannot: its form is 300 tall on a 470-tall panel and does
  not fit above a 330-tall keyboard at any offset. So it cuts the form to what is
  above the keyboard, which turns it into a viewport with more content than room
  — and *that* the tree already knows how to scroll.

Either way, tell the tree afterwards: `Ui::reveal_focused()` re-runs the reveal
when the geometry around the focus changed rather than the focus itself.

## What is not here yet

Key repeat, which needs a press-and-hold signal the toolkit does not have.
