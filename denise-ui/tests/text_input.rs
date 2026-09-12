//! The field, driven the way a person drives it: Tab in, click, double-click,
//! drag, and type over what is selected.

use denise::{
    ElementState, InputEvent, KeyCode, Modifiers, Point, PointerButton, Rect, Size, theme,
};
use denise_ui::widgets::{ClipboardRequest, TextInput};
use denise_ui::{NodeId, TextEngine, TextStyle, Ui};

const SIZE: Size = Size::new(400, 120);
const FIELD: Rect = Rect::new(10, 10, 380, 34);
const STYLE: TextStyle = TextStyle::built_in(16);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Msg {
    Submitted,
    Clipboard(ClipboardRequest),
}

fn key(code: KeyCode) -> InputEvent {
    key_with(code, Modifiers::NONE)
}

fn key_with(code: KeyCode, modifiers: Modifiers) -> InputEvent {
    InputEvent::Key {
        code,
        state: ElementState::Down,
        repeat: false,
        modifiers,
    }
}

/// `times` presses of the same key: `InputEvent` is not `Copy`, so no `[e; n]`.
fn keys(code: KeyCode, modifiers: Modifiers, times: usize) -> Vec<InputEvent> {
    (0..times).map(|_| key_with(code, modifiers)).collect()
}

fn text(s: &str) -> Vec<InputEvent> {
    s.chars().map(|ch| InputEvent::Text { ch }).collect()
}

fn press(at: Point) -> InputEvent {
    press_with(at, Modifiers::NONE)
}

fn press_with(at: Point, modifiers: Modifiers) -> InputEvent {
    InputEvent::PointerButton {
        button: PointerButton::Left,
        state: ElementState::Down,
        position: at,
        modifiers,
    }
}

fn release(at: Point) -> InputEvent {
    InputEvent::PointerButton {
        button: PointerButton::Left,
        state: ElementState::Up,
        position: at,
        modifiers: Modifiers::NONE,
    }
}

fn moved(at: Point) -> InputEvent {
    InputEvent::PointerMoved { position: at }
}

fn field(initial: &str) -> (Ui<Msg>, NodeId) {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let id = ui
        .add(
            root,
            TextInput::<Msg>::new()
                .with_submit(Msg::Submitted)
                .with_clipboard(Msg::Clipboard)
                .with_style(STYLE),
            FIELD,
        )
        .expect("field");
    ui.widget_mut::<TextInput<Msg>>(id)
        .expect("field")
        .set_text(initial);
    (ui, id)
}

fn input(ui: &Ui<Msg>, id: NodeId) -> &TextInput<Msg> {
    ui.widget::<TextInput<Msg>>(id).expect("field")
}

fn messages(ui: &mut Ui<Msg>) -> Vec<Msg> {
    ui.drain_messages().collect()
}

/// The application answering a paste, the way a host with a clipboard does.
fn paste(ui: &mut Ui<Msg>, id: NodeId, text: &str) {
    ui.widget_mut::<TextInput<Msg>>(id)
        .expect("field")
        .insert_text(text);
}

/// The built-in font is fixed-pitch, so a character index has an x.
fn advance() -> i32 {
    TextEngine::new().measure_line(STYLE, "a")
}

/// A point a quarter of the way into character `index`, which is unambiguously
/// inside it rather than on a boundary.
fn at_char(index: i32) -> Point {
    let pad = 16 / 3;
    Point::new(
        FIELD.x + pad + advance() * index + advance() / 4,
        FIELD.y + FIELD.height / 2,
    )
}

#[test]
fn tabbing_in_selects_everything_and_typing_replaces_it() {
    let (mut ui, id) = field("21.5");
    ui.focus(Some(id));
    assert_eq!(
        input(&ui, id).selection(),
        Some((0, 4)),
        "focus offers the value for replacement"
    );
    assert_eq!(input(&ui, id).selected_text(), Some("21.5"));

    ui.handle(&text("7"));
    assert_eq!(input(&ui, id).text(), "7", "typing replaces the selection");
    assert_eq!(input(&ui, id).selection(), None);
    assert_eq!(input(&ui, id).caret(), 1);
}

/// The select-all that focus performs must not outlive the first keystroke:
/// leaving the anchor behind made each typed character replace the one before
/// it, and a field that kept only its last letter.
#[test]
fn typing_after_the_focus_select_keeps_every_character() {
    let (mut ui, id) = field("");
    ui.focus(Some(id));
    ui.handle(&text("Kjærlighet"));
    assert_eq!(input(&ui, id).text(), "Kjærlighet");
    assert_eq!(input(&ui, id).selection(), None);

    // And the same with something to replace first.
    let (mut ui, id) = field("old");
    ui.focus(Some(id));
    ui.handle(&text("new"));
    assert_eq!(input(&ui, id).text(), "new");
}

#[test]
fn focus_from_a_click_lands_where_the_finger_did() {
    let (mut ui, id) = field("hello brave world");
    // No `ui.focus`: the press is what takes the focus, and the select-all that
    // focus performs must not survive the press that caused it.
    ui.handle(&[press(at_char(3)), release(at_char(3))]);
    assert_eq!(input(&ui, id).selection(), None);
    assert_eq!(input(&ui, id).caret(), 3);
}

#[test]
fn a_second_press_takes_the_word_and_a_third_takes_the_field() {
    let (mut ui, id) = field("hello brave world");
    ui.focus(Some(id));
    let at = at_char(8); // inside "brave"

    ui.handle(&[press(at), release(at)]);
    assert_eq!(input(&ui, id).selection(), None, "one press is a caret");

    ui.handle(&[press(at), release(at)]);
    assert_eq!(input(&ui, id).selected_text(), Some("brave"));

    ui.handle(&[press(at), release(at)]);
    assert_eq!(input(&ui, id).selected_text(), Some("hello brave world"));

    // A fourth starts the count again rather than sticking on everything.
    ui.handle(&[press(at), release(at)]);
    assert_eq!(input(&ui, id).selection(), None);
}

#[test]
fn a_press_after_the_window_is_a_fresh_press() {
    let (mut ui, id) = field("hello brave world");
    ui.focus(Some(id));
    let at = at_char(8);
    ui.handle(&[press(at), release(at)]);
    // Longer than DOUBLE_CLICK_MS, so the pair is broken.
    ui.tick(1_000);
    ui.handle(&[press(at), release(at)]);
    assert_eq!(
        input(&ui, id).selection(),
        None,
        "a slow second press is another caret, not a word"
    );
}

#[test]
fn dragging_extends_from_where_the_press_landed() {
    let (mut ui, id) = field("hello brave world");
    ui.focus(Some(id));
    ui.handle(&[press(at_char(6)), moved(at_char(11)), release(at_char(11))]);
    assert_eq!(input(&ui, id).selected_text(), Some("brave"));
    // The release ends the drag: moving afterwards is just a pointer moving.
    ui.handle(&[moved(at_char(2))]);
    assert_eq!(input(&ui, id).selected_text(), Some("brave"));
}

#[test]
fn backspace_and_delete_take_the_selection_first() {
    let (mut ui, id) = field("hello brave world");
    ui.focus(Some(id));
    ui.handle(&[press(at_char(8)), release(at_char(8))]);
    ui.handle(&[press(at_char(8)), release(at_char(8))]);
    ui.handle(&[key(KeyCode::Backspace)]);
    assert_eq!(input(&ui, id).text(), "hello  world");
    assert_eq!(input(&ui, id).caret(), 6);

    // With nothing selected it takes the character before, as it always did.
    ui.handle(&[key(KeyCode::Backspace)]);
    assert_eq!(input(&ui, id).text(), "hello world");

    ui.handle(&[key(KeyCode::Home)]);
    ui.handle(&keys(KeyCode::ArrowRight, Modifiers::SHIFT, 5));
    ui.handle(&[key(KeyCode::Delete)]);
    assert_eq!(input(&ui, id).text(), " world");
}

#[test]
fn shift_extends_and_a_plain_arrow_collapses() {
    let (mut ui, id) = field("abcdef");
    ui.focus(Some(id));
    ui.handle(&[key(KeyCode::Home)]);
    assert_eq!(input(&ui, id).selection(), None, "Home drops the selection");

    ui.handle(&keys(KeyCode::ArrowRight, Modifiers::SHIFT, 3));
    assert_eq!(input(&ui, id).selected_text(), Some("abc"));
    assert_eq!(input(&ui, id).caret(), 3);

    ui.handle(&[key_with(KeyCode::End, Modifiers::SHIFT)]);
    assert_eq!(input(&ui, id).selected_text(), Some("abcdef"));

    ui.handle(&[key(KeyCode::ArrowLeft)]);
    assert_eq!(
        (input(&ui, id).selection(), input(&ui, id).caret()),
        (None, 0),
        "left collapses to the start of the selection, not one back from the caret"
    );

    ui.handle(&keys(KeyCode::ArrowRight, Modifiers::SHIFT, 2));
    ui.handle(&[key(KeyCode::ArrowRight)]);
    assert_eq!(
        (input(&ui, id).selection(), input(&ui, id).caret()),
        (None, 2),
        "and right collapses to the end"
    );
}

#[test]
fn ctrl_a_and_cmd_a_select_everything() {
    for modifier in [Modifiers::CTRL, Modifiers::SUPER] {
        let (mut ui, id) = field("abcdef");
        ui.focus(Some(id));
        ui.handle(&[key(KeyCode::Home)]);
        ui.handle(&[key_with(KeyCode::A, modifier)]);
        assert_eq!(input(&ui, id).selected_text(), Some("abcdef"));
    }
}

#[test]
fn setting_the_text_drops_the_selection() {
    let (mut ui, id) = field("abcdef");
    ui.focus(Some(id));
    assert!(input(&ui, id).selection().is_some());
    ui.widget_mut::<TextInput<Msg>>(id)
        .expect("field")
        .set_text("ghi");
    assert_eq!(input(&ui, id).selection(), None);
    assert_eq!(input(&ui, id).caret(), 3);
}

#[test]
fn a_password_field_selects_the_same_way() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let id = ui
        .add(
            root,
            TextInput::<Msg>::new()
                .with_password(true)
                .with_style(STYLE),
            FIELD,
        )
        .expect("field");
    ui.widget_mut::<TextInput<Msg>>(id)
        .expect("field")
        .set_text("secret");
    ui.focus(Some(id));
    assert_eq!(input(&ui, id).selected_text(), Some("secret"));

    // Stars are one advance each, so a press lands on the same index it would
    // in the clear — the text is hidden, not the geometry.
    ui.handle(&[press(at_char(2)), release(at_char(2))]);
    assert_eq!(input(&ui, id).caret(), 2);
}

#[test]
fn enter_still_submits_and_nothing_else_emits() {
    let (mut ui, id) = field("value");
    ui.focus(Some(id));
    ui.handle(&[press(at_char(1)), release(at_char(1))]);
    ui.handle(&[key(KeyCode::Enter)]);
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Msg::Submitted]
    );
    assert_eq!(input(&ui, id).text(), "value");
}

// ---------------------------------------------------------------- clipboard

#[test]
fn copy_hands_over_the_selection_and_leaves_it_alone() {
    let (mut ui, id) = field("hello brave world");
    ui.focus(Some(id));
    ui.handle(&[press(at_char(8)), release(at_char(8))]);
    ui.handle(&[press(at_char(8)), release(at_char(8))]);
    assert_eq!(input(&ui, id).selected_text(), Some("brave"));

    ui.handle(&[key_with(KeyCode::C, Modifiers::CTRL)]);
    assert_eq!(
        messages(&mut ui),
        vec![Msg::Clipboard(ClipboardRequest::Copy("brave".into()))]
    );
    assert_eq!(
        input(&ui, id).text(),
        "hello brave world",
        "copy edits nothing"
    );
    assert_eq!(input(&ui, id).selected_text(), Some("brave"));
}

#[test]
fn cut_hands_over_the_selection_already_removed() {
    let (mut ui, id) = field("hello brave world");
    ui.focus(Some(id));
    ui.handle(&[press(at_char(8)), release(at_char(8))]);
    ui.handle(&[press(at_char(8)), release(at_char(8))]);

    ui.handle(&[key_with(KeyCode::X, Modifiers::SUPER)]);
    assert_eq!(
        messages(&mut ui),
        vec![Msg::Clipboard(ClipboardRequest::Cut("brave".into()))]
    );
    assert_eq!(input(&ui, id).text(), "hello  world");
    assert_eq!(input(&ui, id).caret(), 6);
    assert_eq!(input(&ui, id).selection(), None);
}

#[test]
fn paste_asks_and_insert_text_answers() {
    let (mut ui, id) = field("ab");
    ui.focus(Some(id));
    ui.handle(&[key(KeyCode::End)]);
    ui.handle(&[key_with(KeyCode::V, Modifiers::CTRL)]);
    assert_eq!(
        messages(&mut ui),
        vec![Msg::Clipboard(ClipboardRequest::Paste)],
        "the widget asks; it has no clipboard of its own"
    );
    assert_eq!(
        input(&ui, id).text(),
        "ab",
        "and nothing happens until answered"
    );

    paste(&mut ui, id, "cd");
    assert_eq!(input(&ui, id).text(), "abcd");
    assert_eq!(input(&ui, id).caret(), 4);

    // A paste replaces what is selected, as typing does.
    ui.handle(&[key(KeyCode::Home)]);
    ui.handle(&keys(KeyCode::ArrowRight, Modifiers::SHIFT, 2));
    paste(&mut ui, id, "ZZ");
    assert_eq!(input(&ui, id).text(), "ZZcd");
}

#[test]
fn a_paste_of_many_lines_takes_the_first() {
    let (mut ui, id) = field("");
    ui.focus(Some(id));
    paste(&mut ui, id, "first line\nsecond line\nthird");
    assert_eq!(input(&ui, id).text(), "first line");

    // Tabs and other control characters are dropped rather than drawn as boxes.
    let (mut ui, id) = field("");
    ui.focus(Some(id));
    paste(&mut ui, id, "a\tb\u{7}c");
    assert_eq!(input(&ui, id).text(), "abc");
}

#[test]
fn a_paste_is_truncated_to_what_the_field_holds() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let id = ui
        .add(
            root,
            TextInput::<Msg>::new().with_max_chars(4).with_style(STYLE),
            FIELD,
        )
        .expect("field");
    ui.widget_mut::<TextInput<Msg>>(id)
        .expect("field")
        .insert_text("abcdefgh");
    assert_eq!(input(&ui, id).text(), "abcd");
    assert_eq!(input(&ui, id).caret(), 4);
}

#[test]
fn a_password_field_refuses_copy_and_cut_and_takes_a_paste() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let id = ui
        .add(
            root,
            TextInput::<Msg>::new()
                .with_password(true)
                .with_clipboard(Msg::Clipboard)
                .with_style(STYLE),
            FIELD,
        )
        .expect("field");
    ui.widget_mut::<TextInput<Msg>>(id)
        .expect("field")
        .set_text("hunter2");
    ui.focus(Some(id));
    assert_eq!(input(&ui, id).selected_text(), Some("hunter2"));

    ui.handle(&[key_with(KeyCode::C, Modifiers::CTRL)]);
    ui.handle(&[key_with(KeyCode::X, Modifiers::CTRL)]);
    assert!(
        messages(&mut ui).is_empty(),
        "a masked value does not reach the clipboard"
    );
    assert_eq!(input(&ui, id).text(), "hunter2", "and cut changed nothing");

    ui.handle(&[key_with(KeyCode::V, Modifiers::CTRL)]);
    assert_eq!(
        messages(&mut ui),
        vec![Msg::Clipboard(ClipboardRequest::Paste)],
        "pasting into one is still allowed"
    );
}

#[test]
fn without_wiring_the_clipboard_keys_do_nothing() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let id = ui
        .add(root, TextInput::<Msg>::new().with_style(STYLE), FIELD)
        .expect("field");
    ui.widget_mut::<TextInput<Msg>>(id)
        .expect("field")
        .set_text("secret plans");
    ui.focus(Some(id));
    ui.handle(&[key_with(KeyCode::C, Modifiers::CTRL)]);
    ui.handle(&[key_with(KeyCode::X, Modifiers::CTRL)]);
    ui.handle(&[key_with(KeyCode::V, Modifiers::CTRL)]);
    assert!(messages(&mut ui).is_empty());
    assert_eq!(
        input(&ui, id).text(),
        "secret plans",
        "and cut with nowhere to go must not delete"
    );
}

#[test]
fn copy_with_nothing_selected_says_nothing() {
    let (mut ui, id) = field("value");
    ui.focus(Some(id));
    ui.handle(&[key(KeyCode::End)]);
    ui.handle(&[key_with(KeyCode::C, Modifiers::CTRL)]);
    ui.handle(&[key_with(KeyCode::X, Modifiers::CTRL)]);
    assert!(messages(&mut ui).is_empty());
    assert_eq!(input(&ui, id).text(), "value");
}
