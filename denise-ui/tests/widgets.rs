//! Behaviour of the shipped widgets, driven the way a user drives them.

use denise::{
    ElementState, InputEvent, KeyCode, Modifiers, Point, PointerButton, Rect, Role, Size, theme,
};
use denise_ui::widgets::{Button, Checkbox, Label, Panel, TextInput};
use denise_ui::{NodeId, Ui};

const SIZE: Size = Size::new(400, 240);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Msg {
    Save,
    Cancel,
    Submitted,
    Logging(bool),
}

fn keys(code: KeyCode, times: usize) -> Vec<InputEvent> {
    (0..times).map(|_| key(code)).collect()
}

fn key(code: KeyCode) -> InputEvent {
    InputEvent::Key {
        code,
        state: ElementState::Down,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

fn text(ch: char) -> InputEvent {
    InputEvent::Text { ch }
}

fn click(x: i32, y: i32) -> [InputEvent; 3] {
    [
        InputEvent::PointerMoved {
            position: Point::new(x, y),
        },
        InputEvent::PointerButton {
            button: PointerButton::Left,
            state: ElementState::Down,
            position: Point::new(x, y),
            modifiers: Modifiers::NONE,
        },
        InputEvent::PointerButton {
            button: PointerButton::Left,
            state: ElementState::Up,
            position: Point::new(x, y),
            modifiers: Modifiers::NONE,
        },
    ]
}

/// A form: a label, a field, and Save/Cancel.
fn form() -> (Ui<Msg>, NodeId, NodeId, NodeId) {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let card = ui
        .add(root, Panel::default(), Rect::new(20, 20, 360, 200))
        .expect("card");
    ui.add(card, Label::new("Name"), Rect::new(20, 16, 120, 24))
        .expect("label");
    let field = ui
        .add(
            card,
            TextInput::<Msg>::new()
                .with_placeholder("Ola Nordmann")
                .with_submit(Msg::Submitted),
            Rect::new(20, 44, 320, 40),
        )
        .expect("field");
    let save = ui
        .add(
            card,
            Button::new("Save", Msg::Save),
            Rect::new(20, 140, 140, 44),
        )
        .expect("save");
    let cancel = ui
        .add(
            card,
            Button::new("Cancel", Msg::Cancel).with_role(Role::Neutral),
            Rect::new(200, 140, 140, 44),
        )
        .expect("cancel");
    (ui, field, save, cancel)
}

#[test]
fn a_button_emits_its_message_when_clicked() {
    let (mut ui, _, _, _) = form();
    // Save sits at card 20,20 plus 20,140 → 40,160, extending 140x44.
    ui.handle(&click(100, 180));
    assert_eq!(ui.drain_messages().collect::<Vec<_>>(), vec![Msg::Save]);
    ui.handle(&click(280, 180));
    assert_eq!(ui.drain_messages().collect::<Vec<_>>(), vec![Msg::Cancel]);
}

#[test]
fn a_focused_button_activates_from_the_keyboard() {
    let (mut ui, _, save, _) = form();
    ui.focus(Some(save));
    ui.handle(&[key(KeyCode::Enter)]);
    ui.handle(&[key(KeyCode::Space)]);
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Msg::Save, Msg::Save],
        "a panel with no pointer has to be drivable by Tab and Enter alone"
    );
}

#[test]
fn a_disabled_button_neither_clicks_nor_takes_focus() {
    let (mut ui, _, save, _) = form();
    ui.set_enabled(save, false);
    ui.handle(&click(100, 180));
    assert!(ui.messages().is_empty());
    ui.focus(Some(save));
    assert_eq!(ui.focused(), None);
}

#[test]
fn typing_reaches_the_focused_field() {
    let (mut ui, field, _, _) = form();
    ui.focus(Some(field));
    for ch in "Ola".chars() {
        ui.handle(&[text(ch)]);
    }
    assert_eq!(
        ui.widget::<TextInput<Msg>>(field).expect("field").text(),
        "Ola"
    );
}

#[test]
fn editing_respects_character_boundaries_not_bytes() {
    let (mut ui, field, _, _) = form();
    ui.focus(Some(field));
    // Every one of these is two bytes in UTF-8. Byte-indexed editing would
    // either panic or produce mojibake.
    for ch in "Kjærlighet på Øy".chars() {
        ui.handle(&[text(ch)]);
    }
    let get = |ui: &Ui<Msg>| {
        ui.widget::<TextInput<Msg>>(field)
            .expect("field")
            .text()
            .to_string()
    };
    assert_eq!(get(&ui), "Kjærlighet på Øy");

    ui.handle(&[key(KeyCode::Home)]);
    ui.handle(&keys(KeyCode::ArrowRight, 2));
    ui.handle(&[key(KeyCode::Delete)]);
    assert_eq!(get(&ui), "Kjrlighet på Øy", "delete removed the æ");

    ui.handle(&[key(KeyCode::End)]);
    ui.handle(&keys(KeyCode::Backspace, 2));
    assert_eq!(get(&ui), "Kjrlighet på ", "backspace removed the Øy");
    assert_eq!(
        ui.widget::<TextInput<Msg>>(field).expect("field").caret(),
        13
    );
}

#[test]
fn the_caret_cannot_run_past_either_end() {
    let (mut ui, field, _, _) = form();
    ui.focus(Some(field));
    for ch in "ab".chars() {
        ui.handle(&[text(ch)]);
    }
    ui.handle(&keys(KeyCode::ArrowRight, 5));
    assert_eq!(ui.widget::<TextInput<Msg>>(field).expect("f").caret(), 2);
    ui.handle(&keys(KeyCode::ArrowLeft, 5));
    assert_eq!(ui.widget::<TextInput<Msg>>(field).expect("f").caret(), 0);
    ui.handle(&[key(KeyCode::Backspace)]);
    assert_eq!(ui.widget::<TextInput<Msg>>(field).expect("f").text(), "ab");
}

#[test]
fn a_field_stops_at_its_character_limit() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let pin = ui
        .add(
            root,
            TextInput::<Msg>::new()
                .with_max_chars(4)
                .with_password(true),
            Rect::new(20, 20, 200, 40),
        )
        .expect("pin");
    ui.focus(Some(pin));
    for ch in "123456".chars() {
        ui.handle(&[text(ch)]);
    }
    assert_eq!(
        ui.widget::<TextInput<Msg>>(pin).expect("pin").text(),
        "1234"
    );
}

#[test]
fn enter_submits_a_field_and_does_not_fall_through() {
    let (mut ui, field, _, _) = form();
    ui.focus(Some(field));
    ui.handle(&[text('x'), key(KeyCode::Enter)]);
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Msg::Submitted],
        "Enter in a field must submit the field, not the nearest button"
    );
}

#[test]
fn control_characters_are_not_inserted_as_text() {
    let (mut ui, field, _, _) = form();
    ui.focus(Some(field));
    ui.handle(&[text('\n'), text('\t'), text('\u{7}'), text('a')]);
    assert_eq!(ui.widget::<TextInput<Msg>>(field).expect("f").text(), "a");
}

#[test]
fn only_the_focused_field_is_asked_to_blink() {
    let (mut ui, field, _, _) = form();
    ui.tick(0);
    assert_eq!(
        ui.next_wake_ms(),
        None,
        "nothing is focused, so nothing should keep the event loop awake"
    );

    ui.focus(Some(field));
    ui.tick(0);
    let first = ui.next_wake_ms().expect("a focused field blinks");
    assert!(first > 0 && first <= 1000);

    // Between blink edges nothing changes, so no frame is owed.
    ui.render_nothing();
    ui.tick(first - 1);
    assert!(!ui.needs_paint(), "an idle caret must not damage anything");

    ui.tick(first);
    assert!(ui.needs_paint(), "the caret going out must be repainted");

    ui.focus(None);
    ui.tick(first + 10);
    assert_eq!(ui.next_wake_ms(), None, "losing focus stops the timer");
}

#[test]
fn a_label_only_repaints_when_its_text_actually_changes() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let label = ui
        .add(root, Label::new("21.5 °C"), Rect::new(20, 20, 200, 30))
        .expect("label");
    ui.render_nothing();

    // The reading is written every cycle; only a different reading is a repaint.
    let changed = ui
        .widget_mut::<Label>(label)
        .expect("label")
        .update("21.5 °C");
    assert!(!changed);
    ui.render_nothing();

    let changed = ui
        .widget_mut::<Label>(label)
        .expect("label")
        .update("21.6 °C");
    assert!(changed);
    assert!(ui.needs_paint());
}

/// Test-only convenience: consume the pending damage without a surface.
trait Settle {
    fn render_nothing(&mut self);
}

impl Settle for Ui<Msg> {
    fn render_nothing(&mut self) {
        use denise::{BufferAge, Frame, PixelFormat};
        let mut pixels = vec![0u32; (SIZE.width * SIZE.height) as usize];
        let mut frame = Frame::new(
            &mut pixels,
            SIZE,
            SIZE.width,
            PixelFormat::Xrgb8888,
            BufferAge::Frames(1),
        )
        .expect("frame");
        self.paint(&mut frame);
        drop(frame);
        self.presented();
    }
}

/// A font whose characters are deliberately different widths.
///
/// The built-in font is monospace, so it cannot tell a caret placed by
/// *measuring* from one placed by multiplying an index by a constant. This can.
#[derive(Debug, Default)]
struct Proportional {
    scratch: Vec<u8>,
}

impl Proportional {
    /// `i` is narrow, `W` is wide, everything else is in between.
    fn advance_of(ch: char, size_px: u16) -> i32 {
        let units = match ch {
            'i' | 'l' | '.' => 2,
            'W' | 'M' => 14,
            _ => 7,
        };
        (units * i32::from(size_px)) / 16
    }
}

impl denise_text::GlyphSource for Proportional {
    fn name(&self) -> &str {
        "proportional test face"
    }

    fn metrics(&self, size_px: u16) -> denise_text::FontMetrics {
        denise_text::FontMetrics {
            ascent: i32::from(size_px) * 3 / 4,
            descent: i32::from(size_px) / 4,
            line_gap: 0,
        }
    }

    fn glyph_metrics(
        &mut self,
        glyph: denise_text::GlyphId,
        size_px: u16,
    ) -> Option<denise_text::GlyphMetrics> {
        let ch = glyph.as_char()?;
        let advance = Self::advance_of(ch, size_px);
        Some(denise_text::GlyphMetrics {
            advance,
            bearing_x: 0,
            bearing_y: i32::from(size_px) * 3 / 4,
            size: Size::new(advance.max(1) as u32, u32::from(size_px) / 2),
        })
    }

    fn rasterise(
        &mut self,
        glyph: denise_text::GlyphId,
        size_px: u16,
    ) -> Option<denise_text::Rasterised<'_>> {
        let metrics = self.glyph_metrics(glyph, size_px)?;
        let len = (metrics.size.width * metrics.size.height) as usize;
        self.scratch.clear();
        self.scratch.resize(len, 255);
        Some(denise_text::Rasterised {
            metrics,
            coverage: &self.scratch,
            stride: metrics.size.width as usize,
        })
    }

    fn contains(&self, _ch: char) -> bool {
        true
    }
}

#[test]
fn the_caret_is_measured_not_counted() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let font = ui.add_font(Box::new(Proportional::default()));
    let style = denise_ui::TextStyle { font, size_px: 16 };
    let root = ui.root();
    let bounds = Rect::new(20, 20, 320, 40);
    let field = ui
        .add(root, TextInput::<Msg>::new().with_style(style), bounds)
        .expect("field");
    ui.focus(Some(field));

    // `iiiW`: three narrow characters then a wide one. A caret placed by
    // multiplying the index by an advance would step evenly; a measured one steps
    // narrow, narrow, narrow, wide.
    for ch in "iiiW".chars() {
        ui.handle(&[text(ch)]);
    }

    // Walk the caret from the start, asking where it is at each step. The
    // measuring engine is built separately from the tree's so that reading the
    // widget and measuring do not borrow the same object at once; both are given
    // the same face, which the assertion below checks.
    let mut engine = denise_text::TextEngine::new();
    let id = engine.add_font(Box::new(Proportional::default()));
    assert_eq!(
        id, font,
        "the test engine must agree with the tree's font ids"
    );

    ui.handle(&[key(KeyCode::Home)]);
    let mut positions = Vec::new();
    for _ in 0..=4 {
        let widget = ui.widget::<TextInput<Msg>>(field).expect("field");
        positions.push(widget.caret_x(&mut engine, bounds));
        ui.handle(&[key(KeyCode::ArrowRight)]);
    }

    let deltas: Vec<i32> = positions.windows(2).map(|w| w[1] - w[0]).collect();
    assert_eq!(deltas.len(), 4);
    assert!(
        deltas[0] == deltas[1] && deltas[1] == deltas[2],
        "the three narrow characters should advance equally: {deltas:?}"
    );
    assert!(
        deltas[3] > deltas[2] * 3,
        "the wide character should advance much further: {deltas:?}"
    );
}

// ------------------------------------------------------------------- checkbox

/// A checkbox at 20,20 measuring 200x30, alone in a tree.
fn checkbox() -> (Ui<Msg>, NodeId) {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let id = ui
        .add(
            root,
            Checkbox::new("Enable logging", Msg::Logging),
            Rect::new(20, 20, 200, 30),
        )
        .expect("checkbox");
    (ui, id)
}

fn checked(ui: &Ui<Msg>, id: NodeId) -> bool {
    ui.widget::<Checkbox<Msg>>(id).expect("checkbox").checked()
}

/// The message carries the value it became, so an application matches on the new
/// state rather than looking the widget up afterwards.
#[test]
fn a_checkbox_emits_the_value_it_changed_to() {
    let (mut ui, id) = checkbox();

    ui.handle(&click(30, 35));
    assert!(checked(&ui, id));
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Msg::Logging(true)]
    );

    ui.handle(&click(30, 35));
    assert!(!checked(&ui, id));
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Msg::Logging(false)],
        "and the value it became on the way back, not a bare 'it changed'"
    );
}

/// The label is part of the target. A 20-pixel box on its own is not something a
/// finger can hit, which is the whole reason the hit area is the widget rather
/// than the box.
#[test]
fn clicking_the_label_toggles_it_too() {
    let (mut ui, id) = checkbox();
    // 150 is well past the box, inside the label's half of the widget.
    ui.handle(&click(150, 35));
    assert!(checked(&ui, id));
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Msg::Logging(true)]
    );
}

/// Space toggles. Enter does **not**: it belongs to the form's default action,
/// and a checkbox that swallows it is why a dialog stops submitting when focus
/// happens to be sitting on one.
#[test]
fn space_toggles_a_focused_checkbox_and_enter_is_left_alone() {
    let (mut ui, id) = checkbox();
    ui.focus(Some(id));

    ui.handle(&[key(KeyCode::Space)]);
    assert!(checked(&ui, id));
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Msg::Logging(true)]
    );

    ui.handle(&[key(KeyCode::Enter)]);
    assert!(checked(&ui, id), "Enter must not have toggled it");
    assert!(ui.messages().is_empty(), "nor emitted anything");
}

/// A held Space is one toggle, not one per repeat. Autorepeat on a checkbox
/// would otherwise flicker the value dozens of times a second.
#[test]
fn holding_space_does_not_toggle_on_every_repeat() {
    let (mut ui, id) = checkbox();
    ui.focus(Some(id));
    ui.handle(&[
        key(KeyCode::Space),
        InputEvent::Key {
            code: KeyCode::Space,
            state: ElementState::Down,
            repeat: true,
            modifiers: Modifiers::NONE,
        },
        InputEvent::Key {
            code: KeyCode::Space,
            state: ElementState::Down,
            repeat: true,
            modifiers: Modifiers::NONE,
        },
    ]);
    assert!(checked(&ui, id));
    assert_eq!(ui.drain_messages().collect::<Vec<_>>().len(), 1);
}

/// Disabled means inert to both the pointer and the keyboard, and not a tab stop.
#[test]
fn a_disabled_checkbox_neither_toggles_nor_takes_focus() {
    let (mut ui, id) = checkbox();
    ui.set_enabled(id, false);

    ui.handle(&click(30, 35));
    assert!(!checked(&ui, id));
    assert!(ui.messages().is_empty());

    ui.focus(Some(id));
    assert_eq!(ui.focused(), None);
}

/// A press dragged off the widget and released elsewhere is cancelled, the same
/// way a button's is. On a touchscreen this is how a finger that landed on the
/// wrong control gets taken back.
#[test]
fn a_press_dragged_off_the_checkbox_does_not_toggle_it() {
    let (mut ui, id) = checkbox();
    ui.handle(&[
        InputEvent::PointerMoved {
            position: Point::new(30, 35),
        },
        InputEvent::PointerButton {
            button: PointerButton::Left,
            state: ElementState::Down,
            position: Point::new(30, 35),
            modifiers: Modifiers::NONE,
        },
        InputEvent::PointerMoved {
            position: Point::new(30, 200),
        },
        InputEvent::PointerButton {
            button: PointerButton::Left,
            state: ElementState::Up,
            position: Point::new(30, 200),
            modifiers: Modifiers::NONE,
        },
    ]);
    assert!(!checked(&ui, id));
    assert!(ui.messages().is_empty());
}

/// Assigning is not the same as somebody clicking. An application that assigned
/// here and got its own message back would either loop or have to guard against
/// itself.
#[test]
fn setting_a_checkbox_from_the_application_emits_nothing() {
    let (mut ui, id) = checkbox();
    ui.widget_mut::<Checkbox<Msg>>(id)
        .expect("checkbox")
        .set_checked(true);
    assert!(checked(&ui, id));
    assert!(ui.messages().is_empty());
}

/// The drawing, which no behavioural test reaches.
///
/// A checkbox whose `checked` flag flips but whose box looks identical is a
/// control nobody can read, and every test above it would still pass. So: paint
/// both states into a buffer and compare the pixels inside the box.
///
/// The tick is drawn with `draw_line`, which is one pixel wide and has no
/// thickness parameter, so it is faked by drawing the pair several times offset
/// downwards. This is what would catch that fake collapsing to nothing.
#[test]
fn ticking_the_box_actually_changes_the_pixels_in_it() {
    fn paint(checked: bool) -> Vec<u32> {
        use denise::{BufferAge, Frame, PixelFormat};
        let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
        let root = ui.root();
        ui.add(
            root,
            Checkbox::new("Enable logging", Msg::Logging).with_checked(checked),
            Rect::new(20, 20, 200, 30),
        )
        .expect("checkbox");

        let mut pixels = vec![0u32; (SIZE.width * SIZE.height) as usize];
        let mut frame = Frame::new(
            &mut pixels,
            SIZE,
            SIZE.width,
            PixelFormat::Xrgb8888,
            BufferAge::Frames(1),
        )
        .expect("frame");
        ui.paint(&mut frame);
        drop(frame);
        pixels
    }

    let (off, on) = (paint(false), paint(true));
    assert_ne!(off, on, "checking the box changed nothing on screen");

    // Not merely different — different *inside the box*, and by more than a
    // couple of pixels. A one-pixel difference would pass the assert above while
    // being invisible on a panel at arm's length.
    let side = theme::DARK.metrics.size_selector as usize;
    let mut differing = 0;
    for y in 20..20 + side {
        for x in 20..20 + side {
            let i = y * SIZE.width as usize + x;
            if off[i] != on[i] {
                differing += 1;
            }
        }
    }
    assert!(
        differing > side * 2,
        "only {differing} pixels differ inside a {side}x{side} box — the tick is \
         not being drawn, or has collapsed to a hairline"
    );
}
