//! The text area, driven the way a person drives it.

use denise::{
    ElementState, InputEvent, KeyCode, Modifiers, Point, PointerButton, Rect, Size, theme,
};
use denise_ui::widgets::{ClipboardRequest, Pos, TextArea};
use denise_ui::{NodeId, TextEngine, TextStyle, Ui};

const SIZE: Size = Size::new(400, 300);
const AREA: Rect = Rect::new(10, 10, 380, 280);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Msg {
    Changed,
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

fn shift(code: KeyCode) -> InputEvent {
    key_with(code, Modifiers::SHIFT)
}

fn ctrl(code: KeyCode) -> InputEvent {
    key_with(code, Modifiers::CTRL)
}

fn text(s: &str) -> Vec<InputEvent> {
    s.chars().map(|ch| InputEvent::Text { ch }).collect()
}

fn press(at: Point, modifiers: Modifiers) -> InputEvent {
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

fn editor(initial: &str) -> (Ui<Msg>, NodeId) {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let id = ui
        .add(
            root,
            TextArea::<Msg>::from_text(initial)
                .with_change(Msg::Changed)
                .with_clipboard(Msg::Clipboard),
            AREA,
        )
        .expect("editor");
    ui.focus(Some(id));
    (ui, id)
}

fn area(ui: &Ui<Msg>, id: NodeId) -> &TextArea<Msg> {
    ui.widget::<TextArea<Msg>>(id).expect("editor")
}

fn messages(ui: &mut Ui<Msg>) -> Vec<Msg> {
    ui.drain_messages().collect()
}

/// A row's height in the built-in font, so a test can aim a click at a line.
fn row_height() -> i32 {
    TextEngine::new().line_height(TextStyle::built_in(16))
}

#[test]
fn typing_makes_lines_and_the_caret_follows() {
    let (mut ui, id) = editor("");
    ui.handle(&text("ab"));
    ui.handle(&[key(KeyCode::Enter)]);
    ui.handle(&text("c"));
    assert_eq!(area(&ui, id).text(), "ab\nc");
    assert_eq!(area(&ui, id).caret(), Pos::new(1, 1));
    assert_eq!(messages(&mut ui), vec![Msg::Changed; 4]);
}

#[test]
fn arrows_cross_line_ends_and_keep_their_column() {
    let (mut ui, id) = editor("long line\nab\nanother long line");
    ui.handle(&[key(KeyCode::End)]);
    assert_eq!(area(&ui, id).caret(), Pos::new(0, 9));
    ui.handle(&[key(KeyCode::ArrowRight)]);
    assert_eq!(
        area(&ui, id).caret(),
        Pos::new(1, 0),
        "right at a line end wraps"
    );
    ui.handle(&[key(KeyCode::ArrowLeft)]);
    assert_eq!(area(&ui, id).caret(), Pos::new(0, 9), "and left wraps back");

    ui.handle(&[key(KeyCode::ArrowDown)]);
    assert_eq!(area(&ui, id).caret(), Pos::new(1, 2), "a short line clamps");
    ui.handle(&[key(KeyCode::ArrowDown)]);
    assert_eq!(
        area(&ui, id).caret(),
        Pos::new(2, 9),
        "the column comes back on a line long enough for it"
    );
    ui.handle(&[key(KeyCode::ArrowUp), key(KeyCode::ArrowUp)]);
    assert_eq!(area(&ui, id).caret(), Pos::new(0, 9));
    assert!(messages(&mut ui).is_empty(), "moving is not a change");
}

#[test]
fn backspace_and_delete_join_lines_at_their_ends() {
    let (mut ui, id) = editor("ab\ncd");
    ui.handle(&[key(KeyCode::ArrowDown)]);
    assert_eq!(area(&ui, id).caret(), Pos::new(1, 0));
    ui.handle(&[key(KeyCode::Backspace)]);
    assert_eq!(area(&ui, id).text(), "abcd");
    assert_eq!(area(&ui, id).caret(), Pos::new(0, 2));
    ui.handle(&[
        key(KeyCode::Enter),
        key(KeyCode::ArrowUp),
        key(KeyCode::End),
    ]);
    ui.handle(&[key(KeyCode::Delete)]);
    assert_eq!(area(&ui, id).text(), "abcd");
    ui.handle(&[key(KeyCode::Home), key(KeyCode::Backspace)]);
    assert_eq!(area(&ui, id).text(), "abcd", "nothing before the start");
}

#[test]
fn editing_steps_over_whole_characters() {
    let (mut ui, id) = editor("");
    ui.handle(&text("på øy"));
    ui.handle(&[
        key(KeyCode::Home),
        key(KeyCode::ArrowRight),
        key(KeyCode::Delete),
    ]);
    assert_eq!(area(&ui, id).text(), "p øy");
    ui.handle(&[
        key(KeyCode::End),
        key(KeyCode::Backspace),
        key(KeyCode::Backspace),
    ]);
    assert_eq!(area(&ui, id).text(), "p ");
}

#[test]
fn shift_extends_a_selection_that_typing_replaces() {
    let (mut ui, id) = editor("one\ntwo\nthree");
    ui.handle(&[
        key(KeyCode::ArrowRight),
        shift(KeyCode::ArrowDown),
        shift(KeyCode::ArrowRight),
    ]);
    assert_eq!(
        area(&ui, id).selection(),
        Some((Pos::new(0, 1), Pos::new(1, 2)))
    );
    assert_eq!(area(&ui, id).selected_text().as_deref(), Some("ne\ntw"));

    ui.handle(&[key(KeyCode::ArrowLeft)]);
    assert_eq!(
        area(&ui, id).selection(),
        None,
        "a plain arrow collapses it"
    );
    assert_eq!(area(&ui, id).caret(), Pos::new(0, 1), "to its near end");

    ui.handle(&[shift(KeyCode::End), shift(KeyCode::ArrowRight)]);
    ui.handle(&text("X"));
    assert_eq!(area(&ui, id).text(), "oXtwo\nthree");
    assert_eq!(area(&ui, id).caret(), Pos::new(0, 2));
}

#[test]
fn select_all_then_delete_leaves_one_empty_line() {
    let (mut ui, id) = editor("one\ntwo");
    ui.handle(&[ctrl(KeyCode::A), key(KeyCode::Backspace)]);
    assert_eq!(area(&ui, id).text(), "");
    assert_eq!(area(&ui, id).caret(), Pos::ZERO);
    ui.handle(&[key(KeyCode::Backspace)]);
    assert_eq!(area(&ui, id).text(), "", "and nothing more happens");
}

#[test]
fn undo_and_redo_reach_the_document_through_the_keyboard() {
    let (mut ui, id) = editor("base");
    ui.handle(&[key(KeyCode::End)]);
    ui.handle(&text(" more"));
    assert_eq!(area(&ui, id).text(), "base more");
    ui.handle(&[ctrl(KeyCode::Z)]);
    assert_eq!(area(&ui, id).text(), "base");
    assert_eq!(
        area(&ui, id).caret(),
        Pos::new(0, 4),
        "clamped to what is left"
    );
    ui.handle(&[key_with(KeyCode::Z, Modifiers::CTRL | Modifiers::SHIFT)]);
    assert_eq!(area(&ui, id).text(), "base more");
    ui.handle(&[ctrl(KeyCode::Z), ctrl(KeyCode::Y)]);
    assert_eq!(area(&ui, id).text(), "base more");
    // Command works as Control does, for the desktop that uses it.
    ui.handle(&[key_with(KeyCode::Z, Modifiers::SUPER)]);
    assert_eq!(area(&ui, id).text(), "base");
}

#[test]
fn the_clipboard_is_asked_for_through_messages() {
    let (mut ui, id) = editor("copy me");
    ui.handle(&[ctrl(KeyCode::C)]);
    assert!(
        messages(&mut ui).is_empty(),
        "nothing selected, nothing copied"
    );

    ui.handle(&[ctrl(KeyCode::A), ctrl(KeyCode::C)]);
    assert_eq!(
        messages(&mut ui),
        vec![Msg::Clipboard(ClipboardRequest::Copy("copy me".into()))]
    );
    assert_eq!(area(&ui, id).text(), "copy me", "copying keeps the text");

    ui.handle(&[ctrl(KeyCode::A), ctrl(KeyCode::X)]);
    assert_eq!(
        messages(&mut ui),
        vec![
            Msg::Clipboard(ClipboardRequest::Cut("copy me".into())),
            Msg::Changed
        ]
    );
    assert_eq!(area(&ui, id).text(), "");

    ui.handle(&[ctrl(KeyCode::V)]);
    assert_eq!(
        messages(&mut ui),
        vec![Msg::Clipboard(ClipboardRequest::Paste)]
    );
    ui.widget_mut::<TextArea<Msg>>(id)
        .expect("editor")
        .insert_text("pasted\ntext");
    assert_eq!(area(&ui, id).text(), "pasted\ntext");
    assert_eq!(area(&ui, id).caret(), Pos::new(1, 4));
    assert!(
        messages(&mut ui).is_empty(),
        "the answer to a paste is silent"
    );
}

#[test]
fn a_click_places_the_caret_and_a_drag_selects() {
    let (mut ui, id) = editor("first\nsecond\nthird");
    let row = row_height();
    // Past the gutter, on the third row; x far right — short of the
    // scrollbar strip — lands at the line end.
    let third = Point::new(AREA.right() - 20, AREA.y + row * 2 + row / 2);
    ui.handle(&[press(third, Modifiers::NONE), release(third)]);
    assert_eq!(area(&ui, id).caret(), Pos::new(2, 5));

    // The clock moves on between clicks, or the next one is a double-click.
    ui.tick(1_000);
    let first = Point::new(AREA.right() - 20, AREA.y + row / 2);
    ui.handle(&[press(first, Modifiers::NONE), moved(third), release(third)]);
    ui.tick(2_000);
    assert_eq!(
        area(&ui, id).selection(),
        Some((Pos::new(0, 5), Pos::new(2, 5)))
    );
    assert_eq!(
        area(&ui, id).selected_text().as_deref(),
        Some("\nsecond\nthird")
    );

    ui.handle(&[press(first, Modifiers::SHIFT)]);
    assert_eq!(
        area(&ui, id).selection(),
        None,
        "shift-click back to the anchor"
    );
}

#[test]
fn a_read_only_area_moves_but_does_not_change() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let id = ui
        .add(
            root,
            TextArea::<Msg>::from_text("fixed")
                .with_read_only(true)
                .with_change(Msg::Changed),
            AREA,
        )
        .expect("editor");
    ui.focus(Some(id));
    ui.handle(&text("x"));
    ui.handle(&[
        key(KeyCode::Backspace),
        key(KeyCode::Enter),
        key(KeyCode::End),
    ]);
    assert_eq!(area(&ui, id).text(), "fixed");
    assert_eq!(area(&ui, id).caret(), Pos::new(0, 5));
    assert!(messages(&mut ui).is_empty());
}

#[test]
fn the_wheel_scrolls_the_view_and_not_the_caret() {
    let lines: Vec<String> = (1..=100).map(|n| format!("line {n}")).collect();
    let (mut ui, id) = editor(&lines.join("\n"));
    let row = row_height() as f32;
    ui.handle(&[InputEvent::PointerScroll {
        delta_x: 0.0,
        delta_y: row * 5.0,
        position: Point::new(100, 100),
    }]);
    assert_eq!(area(&ui, id).top(), 5);
    assert_eq!(area(&ui, id).caret(), Pos::ZERO);
    ui.handle(&[InputEvent::PointerScroll {
        delta_x: 0.0,
        delta_y: -row * 50.0,
        position: Point::new(100, 100),
    }]);
    assert_eq!(area(&ui, id).top(), 0, "and stops at the top");
}

#[test]
fn the_scrollbar_pages_on_a_click_and_follows_a_dragged_thumb() {
    let lines: Vec<String> = (1..=1000).map(|n| format!("line {n}")).collect();
    let (mut ui, id) = editor(&lines.join("\n"));
    let rows = (AREA.height / row_height()) as usize;
    let bar_x = AREA.right() - 3;

    // Below the thumb, which starts at the top: a page down. The caret is
    // untouched — the scrollbar moves the view, not the place in the text.
    let below = Point::new(bar_x, AREA.bottom() - 10);
    ui.handle(&[press(below, Modifiers::NONE), release(below)]);
    assert_eq!(area(&ui, id).top(), rows);
    assert_eq!(area(&ui, id).caret(), Pos::ZERO);
    ui.tick(1_000);
    ui.handle(&[press(below, Modifiers::NONE), release(below)]);
    assert_eq!(area(&ui, id).top(), rows * 2);

    // The thumb is near the top of the strip now — two pages into a
    // thousand lines puts it a few pixels down, and it is at least twenty
    // tall. Take it and drag it to the bottom, and the view is at the end.
    ui.tick(2_000);
    let thumb = Point::new(bar_x, AREA.y + 18);
    let bottom = Point::new(bar_x, AREA.bottom() + 500);
    ui.handle(&[press(thumb, Modifiers::NONE), moved(bottom)]);
    assert_eq!(
        area(&ui, id).top(),
        1000 - rows,
        "the last line on the last row"
    );
    ui.handle(&[release(bottom)]);
    assert_eq!(area(&ui, id).caret(), Pos::ZERO);

    // Above the thumb, now at the bottom: a page up.
    ui.tick(3_000);
    let above = Point::new(bar_x, AREA.y + 4);
    ui.handle(&[press(above, Modifiers::NONE), release(above)]);
    assert_eq!(area(&ui, id).top(), 1000 - rows * 2);
}

#[test]
fn go_to_centres_the_line_once_the_rows_are_known() {
    let lines: Vec<String> = (1..=1000).map(|n| format!("line {n}")).collect();
    let (mut ui, id) = editor(&lines.join("\n"));
    // Nothing has been painted: the line simply goes to the top.
    ui.widget_mut::<TextArea<Msg>>(id)
        .expect("editor")
        .go_to(499);
    assert_eq!(area(&ui, id).caret(), Pos::new(499, 0));
    assert_eq!(area(&ui, id).top(), 499);

    // A paint records the rows, and the next jump is centred.
    let mut pixels = vec![0u32; (SIZE.width * SIZE.height) as usize];
    let mut frame = denise::Frame::new(
        &mut pixels,
        SIZE,
        SIZE.width,
        denise::PixelFormat::Xrgb8888,
        denise::BufferAge::Undefined,
    )
    .expect("frame");
    ui.paint(&mut frame);
    let rows = (AREA.height / row_height()) as usize;
    assert_eq!(area(&ui, id).visible_rows(), rows);
    ui.widget_mut::<TextArea<Msg>>(id)
        .expect("editor")
        .go_to(700);
    assert_eq!(area(&ui, id).top(), 700 - rows / 2);
    ui.widget_mut::<TextArea<Msg>>(id)
        .expect("editor")
        .go_to(5_000);
    assert_eq!(
        area(&ui, id).caret(),
        Pos::new(999, 0),
        "past the end is the end"
    );
    assert_eq!(area(&ui, id).top(), 1000 - rows);
}

#[test]
fn moving_the_caret_off_screen_scrolls_to_it() {
    let lines: Vec<String> = (1..=100).map(|n| format!("line {n}")).collect();
    let (mut ui, id) = editor(&lines.join("\n"));
    ui.handle(&[ctrl(KeyCode::End)]);
    assert_eq!(area(&ui, id).caret(), Pos::new(99, 8));
    let top = area(&ui, id).top();
    let rows = (AREA.height / row_height()) as usize;
    assert_eq!(top, 100 - rows, "the last line is on the last row");
    ui.handle(&[key(KeyCode::PageUp)]);
    assert_eq!(area(&ui, id).caret().line, 99 - rows);
    assert!(area(&ui, id).top() < top);
    ui.handle(&[ctrl(KeyCode::Home)]);
    assert_eq!(area(&ui, id).top(), 0);
}
