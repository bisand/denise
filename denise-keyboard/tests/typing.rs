//! Typing, end to end, with no display: a keyboard over a text field, and the
//! text that ends up in the field.
//!
//! Headless on purpose. The claim this crate makes is about the events it emits
//! and where they end up, and that is exactly what can be checked without a
//! pixel — which is why it is checked on every machine rather than on the one
//! with a panel attached.

use denise::{KeyCode, Rect, Size, theme};
use denise_keyboard::Keyboard;
use denise_layout::{NORWEGIAN, US};
use denise_ui::widgets::TextInput;
use denise_ui::{NodeId, Ui};

#[derive(Clone, Debug, PartialEq)]
enum Msg {
    Key(KeyCode),
}

const SIZE: Size = Size::new(800, 480);

/// A field, focused, with a keyboard over it.
fn set_up(layout: &'static denise_layout::Layout) -> (Ui<Msg>, Keyboard, NodeId) {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let field = ui
        .add(root, TextInput::new(), Rect::new(20, 20, 400, 40))
        .expect("field");
    ui.focus(Some(field));
    let mut keyboard = Keyboard::new(layout);
    keyboard.open(&mut ui, Msg::Key).expect("shelf");
    ui.tick(1_000);
    (ui, keyboard, field)
}

fn text_of(ui: &Ui<Msg>, field: NodeId) -> String {
    ui.widget::<TextInput<Msg>>(field)
        .expect("a text input")
        .text()
        .to_string()
}

/// Tap keys, get text. The events go through `Ui::handle`, which is the call the
/// hardware path's events arrive through, so the field cannot tell them apart.
#[test]
fn tapping_keys_types_into_the_focused_field() {
    let (mut ui, mut keyboard, field) = set_up(&US);

    for code in [KeyCode::H, KeyCode::E, KeyCode::L, KeyCode::L, KeyCode::O] {
        let events = keyboard.press(code);
        ui.handle(&events);
    }

    assert_eq!(text_of(&ui, field), "hello");
    assert_eq!(
        ui.focused(),
        Some(field),
        "typing moved the caret off the field"
    );
}

/// The composition path, which is the reason this crate uses `denise-layout`
/// rather than a table of its own: two taps, one character.
#[test]
fn a_dead_key_composes_with_the_letter_after_it() {
    let (mut ui, mut keyboard, field) = set_up(&NORWEGIAN);

    // The diaeresis lives on BracketRight in Norwegian, and types nothing yet.
    let events = keyboard.press(KeyCode::BracketRight);
    ui.handle(&events);
    assert_eq!(text_of(&ui, field), "", "a dead key typed something");

    let events = keyboard.press(KeyCode::O);
    ui.handle(&events);
    assert_eq!(text_of(&ui, field), "\u{f6}", "expected a composed ö");
}

/// A mark that cannot combine emits both characters rather than swallowing one.
#[test]
fn a_dead_key_that_cannot_combine_emits_both() {
    let (mut ui, mut keyboard, field) = set_up(&NORWEGIAN);

    ui.handle(&keyboard.press(KeyCode::BracketRight));
    ui.handle(&keyboard.press(KeyCode::Q));
    assert_eq!(text_of(&ui, field), "\u{a8}q");
}

/// The layout decides what a position types, and the position does not move.
#[test]
fn the_same_position_types_what_the_layout_says() {
    let (mut ui, mut keyboard, field) = set_up(&NORWEGIAN);
    ui.handle(&keyboard.press(KeyCode::Semicolon));
    assert_eq!(text_of(&ui, field), "\u{f8}", "Norwegian ø");

    let (mut ui, mut keyboard, field) = set_up(&US);
    ui.handle(&keyboard.press(KeyCode::Semicolon));
    assert_eq!(text_of(&ui, field), ";", "US semicolon");
}

/// Control characters are keys and never text: Enter must not insert anything,
/// and Backspace must delete rather than type.
#[test]
fn enter_types_nothing_and_backspace_deletes() {
    let (mut ui, mut keyboard, field) = set_up(&US);

    ui.handle(&keyboard.press(KeyCode::A));
    ui.handle(&keyboard.press(KeyCode::B));
    assert_eq!(text_of(&ui, field), "ab");

    ui.handle(&keyboard.press(KeyCode::Enter));
    assert_eq!(text_of(&ui, field), "ab", "Enter inserted a character");

    ui.handle(&keyboard.press(KeyCode::Backspace));
    assert_eq!(text_of(&ui, field), "a", "Backspace did not delete");
}

/// Space is a character like any other.
#[test]
fn space_types_a_space() {
    let (mut ui, mut keyboard, field) = set_up(&US);
    for code in [KeyCode::H, KeyCode::I, KeyCode::Space, KeyCode::U] {
        ui.handle(&keyboard.press(code));
    }
    assert_eq!(text_of(&ui, field), "hi u");
}

/// Opening and closing is the shelf's lifecycle, and the field keeps the caret
/// across both.
#[test]
fn the_field_keeps_focus_across_open_and_close() {
    let (mut ui, mut keyboard, field) = set_up(&US);
    assert!(keyboard.is_open());
    assert_eq!(ui.focused(), Some(field));

    keyboard.close(&mut ui);
    ui.tick(1_500); // slid out and gone
    assert!(!keyboard.is_open());
    assert!(!ui.shelf_open(), "the shelf outlived the keyboard");
    assert_eq!(ui.focused(), Some(field), "closing moved the caret");
}

/// One at a time, and closing a closed keyboard is not an error.
#[test]
fn open_is_refused_while_one_is_up_and_close_is_idempotent() {
    let (mut ui, mut keyboard, _field) = set_up(&US);
    assert!(
        keyboard.open(&mut ui, Msg::Key).is_none(),
        "a second keyboard was allowed"
    );

    keyboard.close(&mut ui);
    ui.tick(1_500);
    keyboard.close(&mut ui); // no panic, no effect
    assert!(!keyboard.is_open());
}

/// Every key inside the shelf, none of them empty, and pixels where the bottom
/// row should be.
///
/// The typing tests above all pass with the keys laid out anywhere at all —
/// including off-screen or at zero width, since `press` never asks the tree
/// where anything is. This is the one that would notice.
#[test]
fn the_keys_land_inside_the_shelf_and_paint() {
    use denise::{BufferAge, Frame, PixelFormat};

    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let mut keyboard = Keyboard::new(&US);
    let shelf = keyboard.open(&mut ui, Msg::Key).expect("shelf");
    ui.tick(1_000); // slid in

    let shelf_bounds = ui.bounds(shelf).expect("shelf bounds");
    assert_eq!(
        shelf_bounds.height,
        Keyboard::height(),
        "the shelf is not the height the keyboard asked for"
    );

    let expected: usize = denise_keyboard::ROWS.iter().map(|r| r.keys.len()).sum();
    assert_eq!(keyboard.keys().len(), expected, "not every key was added");

    for &(_, key) in keyboard.keys() {
        let b = ui.bounds(key).expect("key bounds");
        assert!(b.width > 0 && b.height > 0, "a key has no size: {b:?}");
        assert!(
            b.x >= shelf_bounds.x
                && b.right() <= shelf_bounds.right()
                && b.y >= shelf_bounds.y
                && b.bottom() <= shelf_bounds.bottom(),
            "a key fell outside the shelf: {b:?} not within {shelf_bounds:?}"
        );
    }

    // And it actually draws: the space bar's middle is not the background.
    let mut buffer = vec![0u32; (SIZE.width * SIZE.height) as usize];
    let mut frame = Frame::new(
        &mut buffer,
        SIZE,
        SIZE.width,
        PixelFormat::Xrgb8888,
        BufferAge::Undefined,
    )
    .expect("frame");
    ui.paint(&mut frame);
    drop(frame);
    let probe = shelf_bounds.y + shelf_bounds.height - denise_keyboard::KEY_HEIGHT / 2;
    let row: Vec<u32> = (0..SIZE.width as i32)
        .map(|x| buffer[(probe * SIZE.width as i32 + x) as usize])
        .collect();
    assert!(
        row.iter().any(|&px| px != row[0]),
        "the bottom key row painted nothing but a flat colour"
    );
}

/// The panel's ordinary behaviour: touch a field and the keyboard arrives,
/// touch something else and it leaves.
#[test]
fn following_focus_opens_on_a_field_and_closes_off_it() {
    use denise_ui::widgets::Button;

    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let field = ui
        .add(root, TextInput::new(), Rect::new(20, 20, 400, 40))
        .expect("field");
    let button = ui
        .add(
            root,
            Button::new("Save", Msg::Key(KeyCode::Enter)),
            Rect::new(20, 80, 120, 40),
        )
        .expect("button");
    ui.tick(1_000);
    let mut keyboard = Keyboard::new(&US);

    // Nothing focused yet: nothing to do.
    keyboard.follow_focus(&mut ui, Msg::Key);
    assert!(!keyboard.is_open());

    ui.focus(Some(field));
    keyboard.follow_focus(&mut ui, Msg::Key);
    assert!(keyboard.is_open(), "focusing a field did not open it");
    ui.tick(1_250);

    // Type something, so the next assertion has something to protect.
    ui.handle(&keyboard.press(KeyCode::H));
    ui.handle(&keyboard.press(KeyCode::I));
    assert_eq!(text_of(&ui, field), "hi");

    // A key press moves no focus, so following focus again changes nothing.
    keyboard.follow_focus(&mut ui, Msg::Key);
    assert!(keyboard.is_open(), "typing closed the keyboard");

    ui.focus(Some(button));
    keyboard.follow_focus(&mut ui, Msg::Key);
    assert!(!keyboard.is_open(), "focusing a button did not close it");
    ui.tick(2_000);
    assert!(!ui.shelf_open());

    // And the text survived both.
    assert_eq!(text_of(&ui, field), "hi");
}

/// Moving between two fields keeps one keyboard up rather than closing and
/// reopening it.
#[test]
fn following_focus_between_two_fields_keeps_it_up() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let first = ui
        .add(root, TextInput::new(), Rect::new(20, 20, 300, 40))
        .expect("first");
    let second = ui
        .add(root, TextInput::new(), Rect::new(20, 80, 300, 40))
        .expect("second");
    ui.tick(1_000);
    let mut keyboard = Keyboard::new(&US);

    ui.focus(Some(first));
    keyboard.follow_focus(&mut ui, Msg::Key);
    ui.tick(1_250);
    let shelf = ui.shelf_open();

    ui.focus(Some(second));
    keyboard.follow_focus(&mut ui, Msg::Key);
    assert!(keyboard.is_open(), "moving between fields closed it");
    assert_eq!(ui.shelf_open(), shelf, "the shelf was rebuilt");

    ui.handle(&keyboard.press(KeyCode::X));
    assert_eq!(text_of(&ui, second), "x", "typing went to the wrong field");
    assert_eq!(text_of(&ui, first), "");
}
