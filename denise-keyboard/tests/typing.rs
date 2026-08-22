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

/// The middle of a node, where a finger would land.
fn middle_of(ui: &Ui<Msg>, node: NodeId) -> denise::Point {
    let b = ui.bounds(node).expect("bounds");
    denise::Point::new(b.x + b.width / 2, b.y + b.height / 2)
}

/// A finger going down at a point.
fn press_at(ui: &mut Ui<Msg>, at: denise::Point, now_ms: u64) {
    use denise::{ElementState, InputEvent, Modifiers, PointerButton};
    ui.tick(now_ms);
    ui.handle(&[
        InputEvent::PointerMoved { position: at },
        InputEvent::PointerButton {
            button: PointerButton::Left,
            state: ElementState::Down,
            position: at,
            modifiers: Modifiers::NONE,
        },
    ]);
}

/// And coming back up again.
fn release_at(ui: &mut Ui<Msg>, at: denise::Point, now_ms: u64) {
    use denise::{ElementState, InputEvent, Modifiers, PointerButton};
    ui.tick(now_ms);
    ui.handle(&[InputEvent::PointerButton {
        button: PointerButton::Left,
        state: ElementState::Up,
        position: at,
        modifiers: Modifiers::NONE,
    }]);
}

/// The node a position sits on, by the code its key carries.
fn key_node(keyboard: &Keyboard, code: KeyCode) -> NodeId {
    keyboard
        .keys()
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, node)| *node)
        .expect("no such key in the grid")
}

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
        keyboard.height(),
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

/// A one-shot shift capitalises exactly one letter and then lets go.
#[test]
fn shift_once_capitalises_one_letter() {
    let (mut ui, mut keyboard, field) = set_up(&US);
    assert_eq!(keyboard.shift(), denise_keyboard::Shift::Off);

    keyboard.press_key(&mut ui, KeyCode::ShiftLeft);
    assert_eq!(keyboard.shift(), denise_keyboard::Shift::Once);

    let events = keyboard.press_key(&mut ui, KeyCode::O);
    ui.handle(&events);
    let events = keyboard.press_key(&mut ui, KeyCode::L);
    ui.handle(&events);
    let events = keyboard.press_key(&mut ui, KeyCode::A);
    ui.handle(&events);

    assert_eq!(
        text_of(&ui, field),
        "Ola",
        "shift did not release after one"
    );
    assert_eq!(keyboard.shift(), denise_keyboard::Shift::Off);
}

/// Caps Lock holds until it is turned off, and leaves the digit row alone —
/// the bug that makes a locked keyboard type `!` for `1`.
///
/// It is a key of its own now rather than a third state of Shift, which is how
/// the keyboard it is shaped after does it and what lets the two combine the
/// way a hand expects.
#[test]
fn caps_lock_holds_and_spares_the_digits() {
    let (mut ui, mut keyboard, field) = set_up(&US);

    keyboard.press_key(&mut ui, KeyCode::CapsLock);
    assert!(keyboard.caps());
    assert_eq!(
        keyboard.shift(),
        denise_keyboard::Shift::Off,
        "caps is not a shift"
    );

    for code in [KeyCode::A, KeyCode::B, KeyCode::Digit1] {
        let events = keyboard.press_key(&mut ui, code);
        ui.handle(&events);
    }
    assert_eq!(
        text_of(&ui, field),
        "AB1",
        "caps lock reached the digit row"
    );

    keyboard.press_key(&mut ui, KeyCode::CapsLock);
    assert!(!keyboard.caps());
    let events = keyboard.press_key(&mut ui, KeyCode::C);
    ui.handle(&events);
    assert_eq!(text_of(&ui, field), "AB1c");
}

/// Caps on plus Shift gives lower case, as on a real keyboard.
#[test]
fn shift_over_caps_lock_types_lower_case() {
    let (mut ui, mut keyboard, field) = set_up(&US);
    keyboard.press_key(&mut ui, KeyCode::CapsLock);
    keyboard.press_key(&mut ui, KeyCode::ShiftLeft);
    let events = keyboard.press_key(&mut ui, KeyCode::A);
    ui.handle(&events);
    assert_eq!(text_of(&ui, field), "a", "caps and shift did not cancel");
}

/// Ctrl arms for exactly one key, and reports itself on that key's events.
#[test]
fn ctrl_is_a_one_shot_that_reaches_the_event() {
    use denise::{ElementState, InputEvent, Modifiers};

    let (mut ui, mut keyboard, _field) = set_up(&US);
    keyboard.press_key(&mut ui, KeyCode::ControlLeft);
    assert!(keyboard.ctrl());

    let events = keyboard.press_key(&mut ui, KeyCode::A);
    assert!(
        events.iter().any(|e| matches!(
            e,
            InputEvent::Key {
                state: ElementState::Down,
                modifiers,
                ..
            } if modifiers.contains(Modifiers::CTRL)
        )),
        "Ctrl did not reach the key event"
    );
    assert!(
        !keyboard.ctrl(),
        "Ctrl stayed armed after the key it modified"
    );
}

/// Shifted keys report the modifier, so a binding on Shift+Enter fires from the
/// on-screen keyboard exactly as it would from a real one.
#[test]
fn a_shifted_key_reports_the_modifier() {
    use denise::{ElementState, InputEvent, Modifiers};

    let (mut ui, mut keyboard, _field) = set_up(&US);
    keyboard.press_key(&mut ui, KeyCode::ShiftLeft);
    let events = keyboard.press_key(&mut ui, KeyCode::Enter);

    let down = events
        .iter()
        .find(|e| {
            matches!(
                e,
                InputEvent::Key {
                    state: ElementState::Down,
                    ..
                }
            )
        })
        .expect("a key down");
    match down {
        InputEvent::Key { modifiers, .. } => {
            assert!(
                modifiers.contains(Modifiers::SHIFT),
                "the shift was not reported"
            );
        }
        _ => unreachable!(),
    }
}

/// The third level is the layout's own, which is the whole reason it is not a
/// grid of symbols chosen here: `@` is AltGr+2 on Norwegian.
#[test]
fn the_third_level_types_the_layouts_own_symbols() {
    let (mut ui, mut keyboard, field) = set_up(&NORWEGIAN);

    keyboard.press_key(&mut ui, KeyCode::AltRight);
    assert!(keyboard.level3());
    let events = keyboard.press_key(&mut ui, KeyCode::Digit2);
    ui.handle(&events);
    assert_eq!(text_of(&ui, field), "@", "AltGr+2 is @ on Norwegian");

    keyboard.press_key(&mut ui, KeyCode::AltRight);
    assert!(!keyboard.level3());
    let events = keyboard.press_key(&mut ui, KeyCode::Digit2);
    ui.handle(&events);
    assert_eq!(text_of(&ui, field), "@2", "the level did not let go");
}

/// A modifier key sends nothing itself: it changes what the next press means.
#[test]
fn a_modifier_key_emits_no_events() {
    let (mut ui, mut keyboard, _field) = set_up(&US);
    assert!(keyboard.press_key(&mut ui, KeyCode::ShiftLeft).is_empty());
    assert!(keyboard.press_key(&mut ui, KeyCode::AltRight).is_empty());
}

/// The keys say what pressing them would produce, at whatever level is showing.
#[test]
fn the_legends_follow_the_level() {
    let (mut ui, mut keyboard, _field) = set_up(&US);

    let label_of = |ui: &Ui<Msg>, keyboard: &Keyboard, want: KeyCode| -> String {
        let (_, node) = keyboard
            .keys()
            .iter()
            .find(|(code, _)| *code == want)
            .copied()
            .expect("the key is in the grid");
        ui.widget::<denise_ui::widgets::Button<Msg>>(node)
            .expect("a button")
            .label()
            .to_string()
    };

    assert_eq!(label_of(&ui, &keyboard, KeyCode::A), "a");
    assert_eq!(label_of(&ui, &keyboard, KeyCode::ShiftLeft), "shift");

    keyboard.press_key(&mut ui, KeyCode::ShiftLeft);
    assert_eq!(
        label_of(&ui, &keyboard, KeyCode::A),
        "A",
        "legend not shifted"
    );
    assert_eq!(label_of(&ui, &keyboard, KeyCode::ShiftLeft), "SHIFT");
    assert_eq!(
        label_of(&ui, &keyboard, KeyCode::Digit1),
        "!",
        "shift does reach the digit row"
    );

    // Spending the one-shot puts the legends back.
    let events = keyboard.press_key(&mut ui, KeyCode::A);
    ui.handle(&events);
    assert_eq!(label_of(&ui, &keyboard, KeyCode::A), "a", "legend stuck");
}

/// Switching relabels the keys where they stand: the position does not move,
/// only what it types.
#[test]
fn switching_layout_reletters_the_same_positions() {
    use denise_layout::{GERMAN, NORWEGIAN};

    let (mut ui, mut keyboard, field) = set_up(&US);
    let label_of = |ui: &Ui<Msg>, keyboard: &Keyboard, want: KeyCode| -> String {
        let (_, node) = keyboard
            .keys()
            .iter()
            .find(|(code, _)| *code == want)
            .copied()
            .expect("in the grid");
        ui.widget::<denise_ui::widgets::Button<Msg>>(node)
            .expect("a button")
            .label()
            .to_string()
    };
    let nodes: Vec<_> = keyboard.keys().to_vec();

    assert_eq!(label_of(&ui, &keyboard, KeyCode::Semicolon), ";");

    keyboard.set_layout(&mut ui, &NORWEGIAN);
    assert_eq!(label_of(&ui, &keyboard, KeyCode::Semicolon), "\u{f8}");
    assert_eq!(
        keyboard.keys(),
        nodes.as_slice(),
        "the grid was rebuilt rather than relettered"
    );

    keyboard.set_layout(&mut ui, &GERMAN);
    assert_eq!(label_of(&ui, &keyboard, KeyCode::Semicolon), "\u{f6}");
    // QWERTZ: the position named Y types z.
    assert_eq!(label_of(&ui, &keyboard, KeyCode::Y), "z");

    // And typing follows the legend.
    let events = keyboard.press_key(&mut ui, KeyCode::Y);
    ui.handle(&events);
    assert_eq!(text_of(&ui, field), "z");
}

/// The layout key walks the built-ins and says where it is.
#[test]
fn the_layout_key_cycles_and_says_which() {
    let (mut ui, mut keyboard, _field) = set_up(&US);
    let layout_key = keyboard
        .keys()
        .iter()
        .map(|(code, _)| *code)
        .find(|code| matches!(code, KeyCode::Unidentified(_)))
        .expect("a layout key in the grid");

    let names: Vec<&str> = denise_layout::BUILT_IN.iter().map(|l| l.name).collect();
    assert_eq!(names, ["us", "no", "de"]);

    assert_eq!(keyboard.layout().name, "us");
    for expected in ["no", "de", "us"] {
        assert!(
            keyboard.press_key(&mut ui, layout_key).is_empty(),
            "the layout key typed something"
        );
        assert_eq!(keyboard.layout().name, expected);
    }
}

/// The globe key still says which layout is live, in its corner.
///
/// It used to say so as its legend. A globe says what the key is *for* and
/// cannot say which of three layouts is current — and on a panel with no other
/// keyboard, "am I typing Norwegian?" is a question only this key can answer.
/// So the name moved to the corner rather than being dropped, and it has to
/// follow the layout the way the legend did.
#[test]
fn the_globe_keeps_the_layout_name_in_its_corner() {
    use denise_ui::widgets::Button;

    let (mut ui, mut keyboard, _field) = set_up(&US);
    let (code, node) = *keyboard
        .keys()
        .iter()
        .find(|(code, _)| matches!(code, KeyCode::Unidentified(_)))
        .expect("a layout key in the grid");

    let corner = |ui: &Ui<Msg>| {
        ui.widget::<Button<Msg>>(node)
            .expect("the layout key is a button")
            .corner()
            .to_string()
    };

    assert_eq!(corner(&ui), "us");
    for expected in ["no", "de", "us"] {
        keyboard.press_key(&mut ui, code);
        assert_eq!(
            corner(&ui),
            expected,
            "the corner did not follow the layout"
        );
    }

    // And the key still reports its name as its label, which is what a test and
    // an accessibility pass read — the globe is drawn over it, not instead.
    assert_eq!(
        ui.widget::<Button<Msg>>(node).unwrap().label(),
        "us",
        "the globe took the key's name away from it"
    );
}

/// A half-typed dead key does not survive a layout change: the mark was waiting
/// for a base character from a layout that has gone.
#[test]
fn switching_layout_drops_a_pending_dead_key() {
    use denise_layout::GERMAN;

    let (mut ui, mut keyboard, field) = set_up(&NORWEGIAN);
    let events = keyboard.press_key(&mut ui, KeyCode::BracketRight); // ¨, held
    ui.handle(&events);
    assert_eq!(text_of(&ui, field), "");

    keyboard.set_layout(&mut ui, &GERMAN);
    let events = keyboard.press_key(&mut ui, KeyCode::O);
    ui.handle(&events);
    assert_eq!(text_of(&ui, field), "o", "the abandoned mark came back");
}

/// Caps Lock is a fact about the keyboard rather than the layout, and survives.
#[test]
fn switching_layout_keeps_caps_lock() {
    use denise_layout::GERMAN;

    let (mut ui, mut keyboard, field) = set_up(&US);
    keyboard.press_key(&mut ui, KeyCode::CapsLock);

    keyboard.set_layout(&mut ui, &GERMAN);
    assert!(keyboard.caps(), "the latch went with the layout");
    let events = keyboard.press_key(&mut ui, KeyCode::Y);
    ui.handle(&events);
    assert_eq!(text_of(&ui, field), "Z", "caps lock did not survive");
}

/// What the keyboard is covering, for a layout that cannot scroll out from
/// under it.
#[test]
fn the_keyboard_says_what_it_covers() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let mut keyboard = Keyboard::new(&US);
    assert_eq!(keyboard.occluded(&ui), None, "nothing is up");

    keyboard.open(&mut ui, Msg::Key).expect("shelf");
    let covered = keyboard.occluded(&ui).expect("the keyboard is up");
    assert_eq!(
        covered.height,
        keyboard.height(),
        "not the height the keyboard asked for"
    );
    assert_eq!(
        covered.bottom(),
        SIZE.height as i32,
        "a keyboard from below reaches the bottom edge"
    );
    ui.tick(1_250);
    assert_eq!(
        keyboard.occluded(&ui),
        Some(covered),
        "moved once it landed"
    );

    keyboard.close(&mut ui);
    assert_eq!(
        keyboard.occluded(&ui),
        None,
        "still covering on the way out"
    );
}

/// A panel at 1.5x is not a panel with smaller fingers.
///
/// The grid is written in logical pixels, so a keyboard that ignored the scale
/// would draw 48-pixel keys on a surface whose every other widget is 72 — half
/// the target, and the half nobody can hit. This asserts the two things that
/// have to hold at any scale: the keys grow with everything else, and they all
/// still fit the shelf they were laid into.
#[test]
fn the_grid_scales_with_the_display() {
    let mut plain: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let mut unscaled = Keyboard::new(&US);
    unscaled.open(&mut plain, Msg::Key).expect("shelf");
    plain.tick(1_000);

    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let mut keyboard = Keyboard::new(&US).with_scale(1.5);
    let shelf = keyboard.open(&mut ui, Msg::Key).expect("shelf");
    ui.tick(1_000); // slid in

    assert_eq!(
        keyboard.height(),
        Keyboard::LOGICAL_HEIGHT * 3 / 2,
        "the shelf did not grow with the display"
    );
    let bounds = ui.bounds(shelf).expect("shelf bounds");
    assert_eq!(bounds.height, keyboard.height());

    // Every key inside the shelf, and taller than it would have been at 1x.
    let plain_key = plain.layout(unscaled.keys()[0].1).expect("key layout");
    for &(_, key) in keyboard.keys() {
        let b = ui.bounds(key).expect("key bounds");
        assert!(
            b.x >= bounds.x
                && b.right() <= bounds.right()
                && b.y >= bounds.y
                && b.bottom() <= bounds.bottom(),
            "a key fell outside the shelf at 1.5x: {b:?} not within {bounds:?}"
        );
        assert!(
            b.height > plain_key.height,
            "a key is no bigger at 1.5x: {} vs {}",
            b.height,
            plain_key.height
        );
    }

    // And the rows still reach the far edge, exactly one gap short of it: a
    // row's leftover width is spent across its keys rather than truncated per
    // key, so eleven keys do not accumulate eleven lost pixels.
    let gap = denise_keyboard::KEY_GAP;
    for (keyboard, ui, scale) in [(&unscaled, &plain, 1.0f32), (&keyboard, &ui, 1.5)] {
        let last = keyboard.keys().last().expect("a key").1;
        let right = ui.bounds(last).expect("bounds").right();
        let want = SIZE.width as i32 - (gap as f32 * scale) as i32;
        assert!(
            (right - want).abs() <= 1,
            "at {scale}x the bottom row ends at {right}, not {want}"
        );
    }
}

/// Legends in the application's own face, because the default cannot be.
///
/// A widget has no way to know which fonts were loaded, so `Button` falls back
/// to the built-in bitmap face — visibly the wrong typeface on a panel that has
/// a real one, and short of glyphs a layout needs.
#[test]
fn the_keys_are_lettered_in_the_style_they_were_given() {
    use denise_text::TextStyle;
    use denise_ui::widgets::Button;

    let style = TextStyle::built_in(28);
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let mut keyboard = Keyboard::new(&US).with_style(style);
    keyboard.open(&mut ui, Msg::Key).expect("shelf");

    for &(code, node) in keyboard.keys() {
        let button = ui.widget::<Button<Msg>>(node).expect("a key is a button");
        assert_eq!(
            button.style().size_px,
            28,
            "{code:?} kept the default style"
        );
    }

    // And a relabel does not quietly put the default back.
    keyboard.tap_shift();
    keyboard.relabel(&mut ui);
    let (_, node) = keyboard.keys()[0];
    let button = ui.widget::<Button<Msg>>(node).expect("a key is a button");
    assert_eq!(button.style().size_px, 28, "relabelling reset the style");
}

/// Holding Backspace keeps deleting.
///
/// The point of the whole feature: clearing a URL bar on a panel was forty
/// taps. The clock is stepped by hand, so this is a claim about time rather
/// than about how fast the machine running the test happens to be.
#[test]
fn holding_backspace_keeps_deleting() {
    let (mut ui, mut keyboard, field) = set_up(&US);
    ui.widget_mut::<TextInput<Msg>>(field)
        .expect("field")
        .set_text("abcdefgh");
    ui.tick(1_000);

    // A finger goes down on Backspace and stays there.
    let back = key_node(&keyboard, KeyCode::Backspace);
    let at = middle_of(&ui, back);
    press_at(&mut ui, at, 1_000);

    // The press itself deletes one, the way every keyboard does.
    let codes: Vec<KeyCode> = ui.drain_messages().map(|Msg::Key(c)| c).collect();
    assert!(!codes.is_empty(), "a repeating key did not emit on press");
    for code in codes {
        let events = keyboard.press_key(&mut ui, code);
        ui.handle(&events);
    }
    assert_eq!(text_of(&ui, field), "abcdefg", "the press did not delete");

    // Nothing yet: still inside the initial pause.
    let early = 1_000 + denise_keyboard::REPEAT_DELAY_MS - 10;
    ui.tick(early);
    let events = keyboard.tick(&mut ui, early);
    assert!(events.is_empty(), "it repeated before the delay was up");

    // Now hold it for 600 ms, in frames, and watch the field empty.
    let mut now = 1_000;
    for _ in 0..40 {
        now += 15;
        ui.tick(now);
        let events = keyboard.tick(&mut ui, now);
        ui.handle(&events);
    }
    assert!(
        text_of(&ui, field).len() < 5,
        "holding Backspace deleted almost nothing: {:?}",
        text_of(&ui, field)
    );

    // And it stops when the finger does.
    release_at(&mut ui, at, now);
    let settled = text_of(&ui, field);
    for _ in 0..20 {
        now += 15;
        ui.tick(now);
        let events = keyboard.tick(&mut ui, now);
        ui.handle(&events);
    }
    assert_eq!(
        text_of(&ui, field),
        settled,
        "it kept deleting after release"
    );
}

/// A repeat says it is one.
///
/// `InputEvent::Key` carries `repeat` so that a widget can tell an auto-repeat
/// from a deliberate second press. A field inserts both; something that must
/// not act twice on one gesture needs the difference.
#[test]
fn a_repeat_is_marked_as_one_and_a_press_is_not() {
    use denise::{ElementState, InputEvent};

    let (_ui, mut keyboard, _field) = set_up(&US);
    let first = keyboard.press(KeyCode::Backspace);
    assert!(
        first.iter().any(|e| matches!(
            e,
            InputEvent::Key {
                state: ElementState::Down,
                repeat: false,
                ..
            }
        )),
        "the first press claimed to be a repeat"
    );

    let repeat = keyboard.press_repeat(KeyCode::Backspace);
    assert!(
        repeat
            .iter()
            .all(|e| !matches!(e, InputEvent::Key { repeat: false, .. })),
        "a repeat did not say so"
    );
}

/// Letters do not repeat, which is what stops a slow finger typing `aaaaaa`.
#[test]
fn holding_a_letter_types_it_once() {
    let (mut ui, mut keyboard, field) = set_up(&US);
    ui.tick(1_000);

    let a = key_node(&keyboard, KeyCode::A);
    let at = middle_of(&ui, a);
    press_at(&mut ui, at, 1_000);

    // Held for a good long while.
    let mut now = 1_000;
    for _ in 0..80 {
        now += 15;
        ui.tick(now);
        let events = keyboard.tick(&mut ui, now);
        ui.handle(&events);
    }
    assert_eq!(text_of(&ui, field), "", "an ordinary key acts on release");

    // And on release it types exactly one.
    release_at(&mut ui, at, now);
    let codes: Vec<KeyCode> = ui.drain_messages().map(|Msg::Key(c)| c).collect();
    for code in codes {
        let events = keyboard.press_key(&mut ui, code);
        ui.handle(&events);
    }
    assert_eq!(text_of(&ui, field), "a", "a held letter did not type once");
}

/// A tree with nothing held asks to be woken for nothing.
///
/// The rule the whole feature had to be built around: a panel that spends its
/// day idle must not pay for a keyboard being on screen.
#[test]
fn a_keyboard_nobody_is_touching_keeps_nobody_awake() {
    // No focused field: a caret blinks, legitimately, and would be the thing
    // keeping the tree awake rather than anything this test is about.
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let mut keyboard = Keyboard::new(&US);
    keyboard.open(&mut ui, Msg::Key).expect("shelf");
    ui.tick(5_000); // past the slide-in
    assert_eq!(
        ui.animating(),
        0,
        "an untouched keyboard is holding the tree awake"
    );
    assert_eq!(ui.next_wake_ms(), None, "and asking to be woken");

    // A finger on Backspace is the one thing that changes that.
    let back = key_node(&keyboard, KeyCode::Backspace);
    let at = middle_of(&ui, back);
    press_at(&mut ui, at, 5_000);
    ui.tick(5_000);
    assert_eq!(ui.animating(), 1, "a held key asked for nothing");
    assert!(ui.next_wake_ms().is_some());

    // And letting go gives it straight back.
    release_at(&mut ui, at, 5_100);
    ui.tick(5_100);
    assert_eq!(ui.animating(), 0, "the tree stayed awake after the release");
    assert_eq!(ui.next_wake_ms(), None);
}

/// A loop that stalled does not empty the field when it comes back.
///
/// The clock is the application's, and applications block: a page arrives over
/// a slow link, a display wakes, a snapshot ticks straight past a second.
/// Counting repeats from the press is truthful about how much time passed and
/// would hand over every one of them at once — which on Backspace means the
/// whole field, because the loop hiccuped.
#[test]
fn a_stalled_loop_does_not_delete_everything_at_once() {
    let (mut ui, mut keyboard, field) = set_up(&US);
    let long = "abcdefghijklmnopqrstuvwxyz".repeat(4);
    ui.widget_mut::<TextInput<Msg>>(field)
        .expect("field")
        .set_text(long.clone());
    ui.tick(1_000);

    let back = key_node(&keyboard, KeyCode::Backspace);
    let at = middle_of(&ui, back);
    press_at(&mut ui, at, 1_000);
    // The press itself deletes one; drop the message, this test is about what
    // comes after it.
    let codes: Vec<KeyCode> = ui.drain_messages().map(|Msg::Key(c)| c).collect();
    for code in codes {
        let events = keyboard.press_key(&mut ui, code);
        ui.handle(&events);
    }
    let before = text_of(&ui, field).chars().count();

    // Ten seconds pass in one frame. At the keyboard's interval that is over a
    // hundred repeats' worth of time.
    let stalled = 11_000;
    ui.tick(stalled);
    let events = keyboard.tick(&mut ui, stalled);
    ui.handle(&events);
    let after = text_of(&ui, field).chars().count();
    let deleted = before - after;
    assert!(
        deleted > 0,
        "the stall swallowed the repeat entirely; it should still be held"
    );
    assert!(
        deleted <= 4,
        "a ten-second stall deleted {deleted} characters in one frame"
    );

    // And it carries on normally from there rather than owing a backlog.
    let mut now = stalled;
    for _ in 0..4 {
        now += 15;
        ui.tick(now);
        let events = keyboard.tick(&mut ui, now);
        ui.handle(&events);
    }
    let steady = after - text_of(&ui, field).chars().count();
    assert!(
        steady <= 2,
        "it kept bursting after the stall: {steady} more in 60 ms"
    );
}

/// Every letter a layout has is reachable from the grid.
///
/// The grid used to be "the positions ISO and ANSI have in common", which
/// sounds careful and quietly dropped `KeyCode::BracketLeft` — where a
/// Norwegian layout keeps **å**. A keyboard for a Norwegian panel that cannot
/// type one of the three Norwegian letters is not a layout question, it is a
/// missing key, and no test asked.
#[test]
fn the_grid_can_reach_every_letter_of_every_layout() {
    use denise_layout::{BUILT_IN, Output};

    let positions: Vec<KeyCode> = denise_keyboard::ROWS
        .iter()
        .flat_map(|row| row.keys)
        .map(|key| key.code)
        .collect();

    for layout in BUILT_IN {
        let mut missing: Vec<char> = Vec::new();
        for entry in layout.entries {
            // Only what a key *types*: a dead key produces nothing on its own
            // and is checked by the composition tests instead.
            let Output::Char(ch) = entry.at(false, false) else {
                continue;
            };
            if ch.is_alphabetic() && !positions.contains(&entry.code) {
                missing.push(ch);
            }
        }
        assert!(
            missing.is_empty(),
            "{} has letters no key can type: {missing:?}",
            layout.name
        );
    }
}

/// The three Norwegian letters, typed on the keys that carry them.
///
/// Named explicitly rather than left to the sweep above, because these are the
/// ones somebody will notice missing and the panel this was built for is
/// Norwegian.
#[test]
fn a_norwegian_panel_can_type_all_three_of_its_letters() {
    let (mut ui, mut keyboard, field) = set_up(&NORWEGIAN);
    for code in [KeyCode::Semicolon, KeyCode::Quote, KeyCode::BracketLeft] {
        let events = keyboard.press(code);
        ui.handle(&events);
    }
    assert_eq!(
        text_of(&ui, field),
        "\u{f8}\u{e6}\u{e5}",
        "ø, æ and å are what a Norwegian keyboard is for"
    );
}

/// Numbers and punctuation say what Shift would give; letters do not.
///
/// A real keyboard prints the `!` above the `1` because you cannot discover
/// Shift by pressing Shift — pressing it is what changes the legend. It prints
/// nothing above the `q`, because a capital Q is not news, and forty keys each
/// carrying a second glyph is a keyboard that reads as noise.
#[test]
fn only_the_number_and_symbol_keys_carry_a_second_legend() {
    use denise_ui::widgets::Button;

    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let mut keyboard = Keyboard::new(&US);
    keyboard.open(&mut ui, Msg::Key).expect("shelf");

    let corner = |ui: &Ui<Msg>, keyboard: &Keyboard, code: KeyCode| {
        let node = key_node(keyboard, code);
        ui.widget::<Button<Msg>>(node)
            .expect("a key is a button")
            .corner()
            .to_string()
    };

    assert_eq!(corner(&ui, &keyboard, KeyCode::Digit1), "!");
    assert_eq!(corner(&ui, &keyboard, KeyCode::Slash), "?");
    assert_eq!(corner(&ui, &keyboard, KeyCode::Equal), "+");

    for letter in [KeyCode::Q, KeyCode::A, KeyCode::Z] {
        assert_eq!(
            corner(&ui, &keyboard, letter),
            "",
            "{letter:?} is a letter and its capital is not news"
        );
    }
    // Keys with a word on them have no other state to advertise.
    for named in [KeyCode::Backspace, KeyCode::Enter, KeyCode::Tab] {
        assert_eq!(corner(&ui, &keyboard, named), "");
    }

    // With Shift held the main legend has already become the shifted
    // character, so printing it again in the corner would say nothing.
    keyboard.press_key(&mut ui, KeyCode::ShiftLeft);
    assert_eq!(
        corner(&ui, &keyboard, KeyCode::Digit1),
        "",
        "the corner repeated what the key now says"
    );
}

/// The corner follows the layout, because the shifted character does.
#[test]
fn the_second_legend_changes_with_the_layout() {
    use denise_ui::widgets::Button;

    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let mut keyboard = Keyboard::new(&US);
    keyboard.open(&mut ui, Msg::Key).expect("shelf");
    let node = key_node(&keyboard, KeyCode::Digit2);
    let corner = |ui: &Ui<Msg>| {
        ui.widget::<Button<Msg>>(node)
            .expect("a key is a button")
            .corner()
            .to_string()
    };

    assert_eq!(corner(&ui), "@", "US puts @ over the 2");
    keyboard.set_layout(&mut ui, &NORWEGIAN);
    assert_eq!(corner(&ui), "\"", "Norwegian puts a quote there instead");
}

/// The named keys draw a picture, whatever font is loaded.
///
/// This is the whole point of drawing rather than looking up. The built-in face
/// carries twenty-three non-ASCII glyphs and not one of `⌫`, `⇥`, `⏎` or a
/// triangle is among them — so on a stock Alpine root, which is exactly the
/// machine this is built for, a glyph legend was a box on the keys nobody can
/// afford to misread. A picture does not care what the font has.
#[test]
fn the_named_keys_are_drawn_whatever_the_font_has() {
    use denise_ui::widgets::Button;

    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let mut keyboard = Keyboard::new(&US);
    keyboard.open(&mut ui, Msg::Key).expect("shelf");

    for code in [
        KeyCode::Backspace,
        KeyCode::Tab,
        KeyCode::Enter,
        KeyCode::ArrowLeft,
        KeyCode::ArrowRight,
    ] {
        let button = ui
            .widget::<Button<Msg>>(key_node(&keyboard, code))
            .expect("a key is a button");
        assert!(button.icon().is_some(), "{code:?} is not drawn");
    }

    // And the built-in face really does lack every glyph they replace, which is
    // the premise the whole thing rests on.
    for ch in [
        '\u{232b}', '\u{21e5}', '\u{23ce}', '\u{25c0}', '\u{25b6}', '\u{2190}',
    ] {
        assert!(
            !ui.text().font_contains(denise_ui::FontId::default(), ch),
            "the built-in face has {ch:?} after all; this test is about the case where it does not"
        );
    }
}

/// A drawn key still reports its word.
///
/// The picture is what somebody sees; the word is what the button reports, and
/// what a test and an accessibility pass read. Losing the second to gain the
/// first would be a bad trade, so the key carries both.
#[test]
fn a_drawn_key_keeps_the_word_it_reports() {
    let at = |code| {
        denise_keyboard::ROWS
            .iter()
            .flat_map(|row| row.keys)
            .find(|key| key.code == code)
            .expect("a key at that position")
    };
    for (code, word) in [
        (KeyCode::Backspace, "back"),
        (KeyCode::Tab, "tab"),
        (KeyCode::Enter, "enter"),
        (KeyCode::ArrowLeft, "<-"),
        (KeyCode::ArrowRight, "->"),
    ] {
        let key = at(code);
        assert!(key.icon.is_some(), "{code:?} should be drawn");
        assert_eq!(key.legend, Some(word), "{code:?} lost the word it reports");
    }

    // The keys whose legend carries state are deliberately not drawn: one glyph
    // cannot say which of two states it is in.
    for code in [KeyCode::ShiftLeft, KeyCode::CapsLock, KeyCode::ControlLeft] {
        assert!(
            at(code).icon.is_none(),
            "{code:?} says which state it is in, and a picture cannot"
        );
    }
}

/// An open keyboard nobody is touching repaints nothing.
///
/// Reported as violent flicker on a Pi with the keyboard up. The cause was in
/// `tick`: collecting what a held key owes went through `Ui::widget_mut`, which
/// damages the node it hands out — "you asked to change it, so assume you did".
/// Asking every key every frame therefore damaged all sixty-odd of them, sixty
/// rectangles overflowed the sixteen the tracker keeps, and the union of them is
/// the whole keyboard. So the keyboard repainted on every frame anything else
/// woke the tree for, and whether that landed above or below `Immediate`'s
/// quarter-of-the-rows threshold decided between a paced flip and an async one
/// that tears.
#[test]
fn an_untouched_keyboard_damages_nothing_per_frame() {
    let (mut ui, mut keyboard, _field) = set_up(&US);
    ui.tick(5_000); // past the slide-in
    ui.presented();

    for step in 1..=8 {
        let now = 5_000 + step * 16;
        ui.tick(now);
        let events = keyboard.tick(&mut ui, now);
        assert!(events.is_empty(), "nothing is held, so nothing repeats");
        let damage: Vec<_> = ui.pending_damage().to_vec();
        assert!(
            damage.is_empty(),
            "frame at {now} ms damaged {} rectangles with nobody touching the \
             keyboard: {damage:?}",
            damage.len()
        );
        ui.presented();
    }
}

/// A finger landing between two keys does not dismiss the keyboard.
///
/// Reported from the panel: an accidental press in the gap put the keyboard
/// away. The gaps are the shelf's backdrop, and an ordinary `Panel` is
/// invisible to hit testing — so the press fell through to the page underneath,
/// and pressing *that* takes the focus off the field being typed into, which is
/// what closes a keyboard that follows focus. A near-miss should cost nothing.
#[test]
fn a_press_in_the_gap_between_keys_changes_nothing() {
    let (mut ui, mut keyboard, field) = set_up(&US);
    ui.tick(5_000);
    assert_eq!(ui.focused(), Some(field));

    // Between two keys on the home row: right of one, left of the next.
    let a = ui.bounds(key_node(&keyboard, KeyCode::A)).expect("bounds");
    let s = ui.bounds(key_node(&keyboard, KeyCode::S)).expect("bounds");
    let gap = denise::Point::new((a.right() + s.x) / 2, a.y + a.height / 2);
    assert!(s.x > a.right(), "the keys should not touch");

    press_at(&mut ui, gap, 5_000);
    release_at(&mut ui, gap, 5_010);

    assert_eq!(
        ui.focused(),
        Some(field),
        "a press in the gap took the focus off the field"
    );
    assert!(keyboard.is_open(), "and put the keyboard away with it");
    assert!(
        ui.drain_messages().next().is_none(),
        "the gap emitted a key press"
    );

    // And typing still works afterwards.
    let events = keyboard.press(KeyCode::A);
    ui.handle(&events);
    assert_eq!(text_of(&ui, field), "a");
}

/// Holding a letter offers its alternates; releasing over one types it.
///
/// The gesture with no precedent in this toolkit. Every other selection here is
/// press-then-release on one node; this one presses a key, opens a strip that
/// was not there when the press began, and is decided by where the finger lifts.
#[test]
fn holding_a_letter_offers_its_alternates_and_the_lift_chooses() {
    let (mut ui, mut keyboard, field) = set_up(&US);
    ui.tick(1_000);

    let o = key_node(&keyboard, KeyCode::O);
    let at = middle_of(&ui, o);
    press_at(&mut ui, at, 1_000);

    // Not yet: an ordinary tap must never open it.
    ui.tick(1_000 + denise_keyboard::HOLD_MS - 50);
    keyboard.tick(&mut ui, 1_000 + denise_keyboard::HOLD_MS - 50);
    assert!(!keyboard.offering(), "it opened before the hold was up");

    let now = 1_000 + denise_keyboard::HOLD_MS + 20;
    ui.tick(now);
    keyboard.tick(&mut ui, now);
    assert!(keyboard.offering(), "holding offered nothing");

    // Slide onto the second choice and lift there.
    let choice = keyboard.choices()[1];
    let over = middle_of(&ui, choice.1);
    ui.handle(&[denise::InputEvent::PointerMoved { position: over }]);
    let typed = keyboard.handle(
        &mut ui,
        &[denise::InputEvent::PointerMoved { position: over }],
    );
    assert!(typed.is_empty(), "moving typed something");

    let up = [denise::InputEvent::PointerButton {
        button: denise::PointerButton::Left,
        state: denise::ElementState::Up,
        position: over,
        modifiers: denise::Modifiers::NONE,
    }];
    let typed = keyboard.handle(&mut ui, &up);
    ui.handle(&up);
    ui.handle(&typed);

    assert!(!keyboard.offering(), "the strip stayed up after the lift");
    assert_eq!(
        text_of(&ui, field),
        choice.0.to_string(),
        "the lift did not choose what it was over"
    );
}

/// Lifting off the strip types nothing at all — not even the key that was held.
#[test]
fn lifting_away_from_the_alternates_types_nothing() {
    let (mut ui, mut keyboard, field) = set_up(&US);
    ui.tick(1_000);
    let o = key_node(&keyboard, KeyCode::O);
    let at = middle_of(&ui, o);
    press_at(&mut ui, at, 1_000);
    let now = 1_000 + denise_keyboard::HOLD_MS + 20;
    ui.tick(now);
    keyboard.tick(&mut ui, now);
    assert!(keyboard.offering());

    // Somewhere that is neither the strip nor the key.
    let away = denise::Point::new(4, 4);
    let up = [denise::InputEvent::PointerButton {
        button: denise::PointerButton::Left,
        state: denise::ElementState::Up,
        position: away,
        modifiers: denise::Modifiers::NONE,
    }];
    let typed = keyboard.handle(&mut ui, &up);
    ui.handle(&up);
    ui.handle(&typed);

    assert!(!keyboard.offering());
    assert_eq!(
        text_of(&ui, field),
        "",
        "an abandoned gesture typed something"
    );
    // And left nothing behind for the application to type later: the key was
    // released outside its own bounds, so it never emitted either.
    assert_eq!(
        ui.drain_messages().count(),
        0,
        "the held key emitted anyway"
    );
}

/// Lifting back onto the key you were holding types that key.
///
/// The escape hatch, and the reason the strip does not repeat the base
/// character among its choices: the key itself is still there under the strip,
/// still where the finger already is, and lifting on it means what lifting on a
/// key has always meant. A finger that opened the strip by accident undoes it by
/// not moving.
#[test]
fn lifting_back_on_the_key_types_the_key() {
    let (mut ui, mut keyboard, field) = set_up(&US);
    ui.tick(1_000);
    let o = key_node(&keyboard, KeyCode::O);
    let at = middle_of(&ui, o);
    press_at(&mut ui, at, 1_000);
    let now = 1_000 + denise_keyboard::HOLD_MS + 20;
    ui.tick(now);
    keyboard.tick(&mut ui, now);
    assert!(keyboard.offering());

    let up = [denise::InputEvent::PointerButton {
        button: denise::PointerButton::Left,
        state: denise::ElementState::Up,
        position: at,
        modifiers: denise::Modifiers::NONE,
    }];
    let typed = keyboard.handle(&mut ui, &up);
    assert!(
        typed.is_empty(),
        "the strip claimed a lift that was not on it"
    );
    ui.handle(&up);
    ui.handle(&typed);

    assert!(!keyboard.offering(), "the strip stayed up");
    let codes: Vec<KeyCode> = ui.drain_messages().map(|Msg::Key(c)| c).collect();
    for code in codes {
        let events = keyboard.press_key(&mut ui, code);
        ui.handle(&events);
    }
    assert_eq!(
        text_of(&ui, field),
        "o",
        "lifting on the key did not type it"
    );
}

/// The whole gesture on glass: touch down, slide, lift.
///
/// The pointer path and the touch path are separate arms, and the panels this
/// keyboard exists for have no mouse — so the arm that will actually carry the
/// gesture in the field is this one. Neither test board has a touchscreen, which
/// makes the unit test the only place it is exercised at all.
#[test]
fn a_finger_can_do_the_whole_gesture() {
    use denise::InputEvent;

    let (mut ui, mut keyboard, field) = set_up(&US);
    ui.tick(1_000);
    let e = key_node(&keyboard, KeyCode::E);
    let at = middle_of(&ui, e);
    let id = 1;

    ui.handle(&[InputEvent::TouchDown { id, position: at }]);
    let now = 1_000 + denise_keyboard::HOLD_MS + 20;
    ui.tick(now);
    keyboard.tick(&mut ui, now);
    assert!(keyboard.offering(), "a finger holding e offered nothing");

    let (wanted, choice) = keyboard.choices()[2];
    let over = middle_of(&ui, choice);
    let moved = [InputEvent::TouchMoved { id, position: over }];
    keyboard.handle(&mut ui, &moved);
    ui.handle(&moved);

    let up = [InputEvent::TouchUp {
        id,
        position: over,
        cancelled: false,
    }];
    let typed = keyboard.handle(&mut ui, &up);
    ui.handle(&up);
    ui.handle(&typed);

    assert!(!keyboard.offering());
    assert_eq!(text_of(&ui, field), wanted.to_string());
}

/// A sequence the system cancels is not a choice, wherever the finger was.
///
/// The distinction the pointer path does not have to make: a palm landing, a
/// gesture claimed by the window system, a digitiser giving up mid-slide. The
/// last reported position can be squarely over a choice and it still must not
/// type one — a character the user did not ask for is worse than none.
#[test]
fn a_cancelled_touch_chooses_nothing() {
    use denise::InputEvent;

    let (mut ui, mut keyboard, field) = set_up(&US);
    ui.tick(1_000);
    let e = key_node(&keyboard, KeyCode::E);
    let at = middle_of(&ui, e);
    let id = 1;

    ui.handle(&[InputEvent::TouchDown { id, position: at }]);
    let now = 1_000 + denise_keyboard::HOLD_MS + 20;
    ui.tick(now);
    keyboard.tick(&mut ui, now);
    assert!(keyboard.offering());

    // Squarely over a choice, and cancelled anyway.
    let over = middle_of(&ui, keyboard.choices()[0].1);
    let up = [InputEvent::TouchUp {
        id,
        position: over,
        cancelled: true,
    }];
    let typed = keyboard.handle(&mut ui, &up);
    ui.handle(&up);
    ui.handle(&typed);

    assert!(!keyboard.offering(), "the strip survived a cancelled touch");
    assert_eq!(
        text_of(&ui, field),
        "",
        "a cancelled gesture typed a character"
    );
    assert_eq!(ui.drain_messages().count(), 0, "and it emitted one too");
}

/// The strip takes its choices with it when it goes.
///
/// They hang off the strip rather than off the shelf precisely so that one
/// `Ui::remove` clears the lot. Parented to the shelf instead they would outlive
/// the gesture and sit there being pressable, which is a bug that looks like
/// nothing until the second time somebody holds a key.
#[test]
fn closing_the_strip_leaves_no_nodes_behind() {
    let (mut ui, mut keyboard, _field) = set_up(&US);
    ui.tick(1_000);
    keyboard.offer_for_test(&mut ui, KeyCode::O);
    let nodes: Vec<NodeId> = keyboard.choices().iter().map(|&(_, n)| n).collect();
    assert!(nodes.len() > 1, "nothing was offered to begin with");
    assert!(nodes.iter().all(|&n| ui.contains(n)));

    let away = denise::Point::new(4, 4);
    let up = [denise::InputEvent::PointerButton {
        button: denise::PointerButton::Left,
        state: denise::ElementState::Up,
        position: away,
        modifiers: denise::Modifiers::NONE,
    }];
    keyboard.handle(&mut ui, &up);

    assert!(!keyboard.offering());
    for node in nodes {
        assert!(
            !ui.contains(node),
            "a choice outlived the strip that made it"
        );
    }
}

/// A key with nothing to offer offers nothing, however long it is held.
#[test]
fn holding_a_key_with_no_alternates_does_nothing() {
    let (mut ui, mut keyboard, field) = set_up(&US);
    ui.tick(1_000);
    // `k` has no accented relatives anybody reaches for.
    let k = key_node(&keyboard, KeyCode::K);
    let at = middle_of(&ui, k);
    press_at(&mut ui, at, 1_000);
    for step in 1..=6 {
        let now = 1_000 + step * denise_keyboard::HOLD_MS;
        ui.tick(now);
        keyboard.tick(&mut ui, now);
    }
    assert!(!keyboard.offering(), "k offered something");

    // And it still types normally when released on.
    release_at(&mut ui, at, 5_000);
    let codes: Vec<KeyCode> = ui.drain_messages().map(|Msg::Key(c)| c).collect();
    for code in codes {
        let events = keyboard.press_key(&mut ui, code);
        ui.handle(&events);
    }
    assert_eq!(text_of(&ui, field), "k", "a held key stopped typing");
}

/// The alternates come from the layout, so switching layout switches them.
#[test]
fn the_alternates_are_the_layouts_own() {
    use denise_layout::{GERMAN, NORWEGIAN, US};

    // Norwegian has æ, ø and å on keys of their own, so it does not offer them
    // again as alternates — a slower way to reach a letter you already have is
    // noise.
    let no: String = NORWEGIAN.alternates_for('o').collect();
    assert!(!no.contains('ø'), "ø has a key; it should not be an offer");
    assert!(no.contains('ö'), "but its neighbours are worth offering");

    let us: String = US.alternates_for('o').collect();
    assert!(us.contains('ø'), "US has no ø key, so it offers one");

    // German offers ß from s, which is where a writer reaches for it.
    let de: String = GERMAN.alternates_for('s').collect();
    assert!(de.contains('ß'));
    assert!(!US.alternates_for('s').any(|c| c == 'ß'));
}

/// Shifted keys offer shifted alternates: hold `O` and you get `Ö`, not `ö`.
#[test]
fn the_offers_follow_the_case_of_the_key() {
    use denise_layout::US;
    let lower: String = US.alternates_for('o').collect();
    let upper: String = US.alternates_for('O').collect();
    assert!(lower.contains('ö') && !lower.contains('Ö'));
    assert!(upper.contains('Ö') && !upper.contains('ö'));
    assert_eq!(lower.chars().count(), upper.chars().count());
}
