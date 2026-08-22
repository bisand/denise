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

// Whatever the machine is configured for; US when it says nothing.
let mut keyboard = Keyboard::new(denise_layout::from_system().0);
keyboard.open(&mut ui, Msg::Key).unwrap();
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

## What is not here yet

Shift, caps lock, a symbol page and key repeat; layout switching from a key on
the keyboard itself. This types letters, digits, space, backspace and enter, in
whatever layout it was given.
