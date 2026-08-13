//! Behaviour of the shipped widgets, driven the way a user drives them.

use denise::{
    ElementState, InputEvent, KeyCode, Modifiers, Point, PointerButton, Rect, Role, Size, theme,
};
use denise_ui::widgets::{
    Alert, Badge, Button, Checkbox, Divider, Label, List, ListItem, Panel, Progress,
    RadialProgress, RadioGroup, Select, Slider, Spinner, Tabs, TextInput, Toggle,
};
use denise_ui::{Animation, NodeId, PaintCtx, Ui, Widget};

const SIZE: Size = Size::new(400, 240);

// No `Eq`: `Level` carries an f32. `assert_eq!` only ever wanted `PartialEq`.
#[derive(Clone, Debug, PartialEq)]
enum Msg {
    Save,
    Cancel,
    Submitted,
    Logging(bool),
    Muted(bool),
    Mode(usize),
    Level(f32),
    Page(usize),
    Row(usize),
    Open(usize),
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
    assert_eq!(ui.animating(), 0);

    ui.focus(Some(field));
    ui.tick(0);
    let first = ui.next_wake_ms().expect("a focused field blinks");
    assert!(first > 0 && first <= 1000);
    assert_eq!(ui.animating(), 1);

    // Between blink edges nothing changes, so no frame is owed.
    ui.render_nothing();
    ui.tick(first - 1);
    assert!(!ui.needs_paint(), "an idle caret must not damage anything");

    ui.tick(first);
    assert!(ui.needs_paint(), "the caret going out must be repainted");

    // Losing focus makes the field answer `None` at the next tick, which is
    // the widget handing the CPU back — not the tree confiscating it.
    ui.focus(None);
    ui.tick(first + 10);
    assert_eq!(ui.next_wake_ms(), None, "losing focus stops the timer");
    assert_eq!(ui.animating(), 0, "and leaves nothing in the animating set");
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

/// The pixels inside `area`, after a paint.
fn pixels_of(ui: &mut Ui<Msg>, area: Rect) -> Vec<u32> {
    use denise::{BufferAge, Frame, PixelFormat};
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
    let mut out = Vec::new();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            out.push(buffer[(y * SIZE.width as i32 + x) as usize]);
        }
    }
    out
}

/// Test-only convenience: consume the pending damage without a surface.
trait Settle {
    fn render_nothing(&mut self);
    /// Paints, and reports what that paint resolved as damage.
    fn paint_for_damage(&mut self) -> Vec<Rect>;
}

impl Settle for Ui<Msg> {
    fn paint_for_damage(&mut self) -> Vec<Rect> {
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
        let damage = self.damage().to_vec();
        self.presented();
        damage
    }

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

// --------------------------------------------------------------------- toggle

/// A toggle at 20,20 measuring 200x30, with a field after it to take focus away.
fn toggled() -> (Ui<Msg>, NodeId, NodeId) {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let id = ui
        .add(
            root,
            Toggle::new("Mute", Msg::Muted),
            Rect::new(20, 20, 200, 30),
        )
        .expect("toggle");
    let field = ui
        .add(root, TextInput::<Msg>::new(), Rect::new(20, 90, 200, 40))
        .expect("field");
    (ui, id, field)
}

fn on(ui: &Ui<Msg>, id: NodeId) -> bool {
    ui.widget::<Toggle<Msg>>(id).expect("toggle").checked()
}

#[test]
fn a_toggle_emits_the_value_it_changed_to() {
    let (mut ui, id, _) = toggled();

    ui.handle(&click(30, 35));
    assert!(on(&ui, id));
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Msg::Muted(true)]
    );

    ui.handle(&click(30, 35));
    assert!(!on(&ui, id));
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Msg::Muted(false)]
    );
}

/// Space toggles, Enter is left for the form's default action — the same rule as
/// `Checkbox`, and worth pinning separately because it is easy to implement one
/// and forget the other.
#[test]
fn space_toggles_a_focused_toggle_and_enter_is_left_alone() {
    let (mut ui, id, _) = toggled();
    ui.focus(Some(id));

    ui.handle(&[key(KeyCode::Space)]);
    assert!(on(&ui, id));

    ui.handle(&[key(KeyCode::Enter)]);
    assert!(on(&ui, id), "Enter must not have toggled it");
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Msg::Muted(true)]
    );
}

/// The animation has to *end*. A widget that keeps asking for frames holds a
/// kiosk's CPU awake for the life of the device, which is the number the README
/// leads with.
#[test]
fn the_knob_animates_and_then_stops_asking_for_frames() {
    let (mut ui, id, _) = toggled();

    ui.tick(0);
    assert_eq!(ui.next_wake_ms(), None, "nothing is animating at rest");

    ui.handle(&click(30, 35));
    assert!(on(&ui, id));

    // Clicking focused it, so it is the widget `tick` will animate.
    assert_eq!(ui.focused(), Some(id));

    ui.tick(0);
    let waking = ui.next_wake_ms();
    assert!(waking.is_some(), "the knob should be crossing");

    // Run the clock past the travel time. The exact number of frames does not
    // matter; that it stops does.
    let mut frames = 0;
    let mut now = 0;
    while ui.next_wake_ms().is_some() && frames < 100 {
        now = ui.next_wake_ms().expect("a wake time");
        ui.tick(now);
        frames += 1;
    }
    assert!(frames > 1, "the knob jumped rather than crossing");
    assert!(
        frames < 100,
        "the knob never settled: still waking at {now}ms"
    );
    assert_eq!(ui.next_wake_ms(), None, "and stops asking once it arrives");
}

/// While it moves, the damage stays inside the toggle. A widget that invalidated
/// its whole scene during an animation would repaint the panel sixty times a
/// second for something 50 pixels wide.
///
/// `Ui::damage` reports what the **last paint** resolved, not what is pending, so
/// this has to paint between the tick and the assertion. Reading it straight
/// after `tick` returns the previous frame's answer, which is the full surface
/// for a tree that has just been built.
#[test]
fn an_animating_knob_only_damages_its_own_rectangle() {
    let (mut ui, id, _) = toggled();
    let bounds = ui.bounds(id).expect("bounds");

    // Settle the initial full-surface damage, and then the click's.
    ui.render_nothing();
    ui.handle(&click(30, 35));
    ui.render_nothing();

    ui.tick(8);
    let damage = ui.paint_for_damage();
    assert!(!damage.is_empty(), "a moving knob has to repaint something");
    for rect in &damage {
        assert!(
            bounds.contains_rect(rect),
            "{rect:?} escaped the toggle's own {bounds:?}"
        );
    }
}

/// The failure #19 existed for: a toggle that loses focus mid-slide used to be
/// stranded, because only the focused widget was ever asked to animate. Now the
/// crossing belongs to the widget, not to focus — it keeps animating to the far
/// end and then stops asking.
///
/// Asserted through the pixels, because there is no public way to ask where the
/// knob is, and "the value is true" would pass with the knob stuck at 40%. The
/// reference is the same toggle built already-on and never animated.
#[test]
fn a_toggle_that_loses_focus_mid_slide_finishes_its_travel() {
    let (mut ui, id, _) = toggled();
    let bounds = ui.bounds(id).expect("bounds");

    ui.handle(&click(30, 35));
    ui.tick(0);
    assert!(ui.next_wake_ms().is_some(), "mid-slide");

    // Clicking the background is what drops focus, and it is the ordinary way a
    // panel loses it. The knob is a third of the way across at this point — and
    // it must keep crossing.
    ui.focus(None);
    ui.tick(40);
    assert_eq!(ui.animating(), 1, "the crossing survives losing focus");
    assert!(
        ui.next_wake_ms().is_some(),
        "an unfocused toggle mid-slide still gets frames"
    );

    // Past the end of the travel it settles and hands the CPU back.
    ui.tick(500);
    assert_eq!(ui.animating(), 0, "arrived, and stopped asking");
    assert_eq!(ui.next_wake_ms(), None, "nothing is animating any more");
    assert!(on(&ui, id), "the value was never in doubt");

    // The pointer is still sitting on the toggle, and a hovered track is a
    // different colour from a resting one. Move it off, or this compares hover
    // states rather than knob positions.
    ui.handle(&[InputEvent::PointerMoved {
        position: Point::new(380, 220),
    }]);
    let stranded = pixels_of(&mut ui, bounds);

    let mut reference: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = reference.root();
    reference
        .add(
            root,
            Toggle::new("Mute", Msg::Muted).with_checked(true),
            Rect::new(20, 20, 200, 30),
        )
        .expect("toggle");
    let landed = pixels_of(&mut reference, bounds);

    let differing = stranded.iter().zip(&landed).filter(|(a, b)| a != b).count();
    assert_eq!(
        differing,
        0,
        "{differing} of {} pixels differ: the knob froze part-way instead of \
         finishing its travel",
        stranded.len()
    );
}

/// Flipping it back mid-slide reverses from where the knob got to.
#[test]
fn flipping_a_toggle_back_mid_slide_does_not_jump() {
    let (mut ui, id, _) = toggled();

    ui.handle(&click(30, 35));
    ui.tick(0);
    ui.tick(40);
    ui.handle(&click(30, 35));
    assert!(!on(&ui, id));

    // Still animating, and still bounded — the reversal is a new transition, not
    // a second one layered on the first.
    ui.tick(40);
    assert!(ui.next_wake_ms().is_some());
    let mut frames = 0;
    while ui.next_wake_ms().is_some() && frames < 100 {
        let now = ui.next_wake_ms().expect("a wake time");
        ui.tick(now);
        frames += 1;
    }
    assert!(frames < 100, "the reversed transition never ended");
}

#[test]
fn a_disabled_toggle_neither_toggles_nor_takes_focus() {
    let (mut ui, id, _) = toggled();
    ui.set_enabled(id, false);

    ui.handle(&click(30, 35));
    assert!(!on(&ui, id));
    assert!(ui.messages().is_empty());

    ui.focus(Some(id));
    assert_eq!(ui.focused(), None);
}

#[test]
fn setting_a_toggle_from_the_application_emits_nothing() {
    let (mut ui, id, _) = toggled();
    ui.widget_mut::<Toggle<Msg>>(id)
        .expect("toggle")
        .set_checked(true);
    assert!(on(&ui, id));
    assert!(ui.messages().is_empty());
}

// ---------------------------------------------------------------- radio group

/// A three-option group at 20,20 measuring 200x90 — rows of 30 — with a button
/// before it and after it, so Tab has somewhere to come from and go to.
fn radios() -> (Ui<Msg>, NodeId, NodeId, NodeId) {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let before = ui
        .add(
            root,
            Button::new("Before", Msg::Save),
            Rect::new(20, 200, 100, 30),
        )
        .expect("before");
    let group = ui
        .add(
            root,
            RadioGroup::new(["Auto", "Manual", "Off"], Msg::Mode),
            Rect::new(20, 20, 200, 90),
        )
        .expect("group");
    let after = ui
        .add(
            root,
            Button::new("After", Msg::Cancel),
            Rect::new(140, 200, 100, 30),
        )
        .expect("after");
    (ui, before, group, after)
}

fn mode(ui: &Ui<Msg>, id: NodeId) -> usize {
    ui.widget::<RadioGroup<Msg>>(id).expect("group").selected()
}

/// Clicking a row chooses it, and the message carries the index.
#[test]
fn clicking_an_option_chooses_it() {
    let (mut ui, _, group, _) = radios();

    ui.handle(&click(60, 65)); // the middle row, y 50..80
    assert_eq!(mode(&ui, group), 1);
    assert_eq!(ui.drain_messages().collect::<Vec<_>>(), vec![Msg::Mode(1)]);

    ui.handle(&click(60, 95)); // the last row, y 80..110
    assert_eq!(mode(&ui, group), 2);
    assert_eq!(ui.drain_messages().collect::<Vec<_>>(), vec![Msg::Mode(2)]);
}

/// Re-choosing what is already chosen is not a change, so it emits nothing —
/// otherwise an application redoes its work for a click that meant nothing.
#[test]
fn clicking_the_chosen_option_again_emits_nothing() {
    let (mut ui, _, group, _) = radios();
    ui.handle(&click(60, 35));
    assert_eq!(mode(&ui, group), 0);
    assert!(
        ui.messages().is_empty(),
        "option 0 was already chosen; nothing changed"
    );
}

/// Arrows move the choice and wrap, in both directions. This is what makes a
/// panel with no pointer usable.
#[test]
fn arrows_move_the_choice_and_wrap() {
    let (mut ui, _, group, _) = radios();
    ui.focus(Some(group));

    ui.handle(&[key(KeyCode::ArrowDown)]);
    assert_eq!(mode(&ui, group), 1);
    ui.handle(&[key(KeyCode::ArrowDown)]);
    assert_eq!(mode(&ui, group), 2);
    ui.handle(&[key(KeyCode::ArrowDown)]);
    assert_eq!(mode(&ui, group), 0, "past the end comes back to the start");

    ui.handle(&[key(KeyCode::ArrowUp)]);
    assert_eq!(mode(&ui, group), 2, "and before the start goes to the end");

    // Right and Left do the same, since a horizontal-looking group is still one
    // list to the keyboard.
    ui.handle(&[key(KeyCode::ArrowRight)]);
    assert_eq!(mode(&ui, group), 0);
    ui.handle(&[key(KeyCode::ArrowLeft)]);
    assert_eq!(mode(&ui, group), 2);

    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![
            Msg::Mode(1),
            Msg::Mode(2),
            Msg::Mode(0),
            Msg::Mode(2),
            Msg::Mode(0),
            Msg::Mode(2),
        ],
        "every arrow that moved the choice reported it, once"
    );
}

/// **The rule this widget exists for.** Tab moves past the whole group, not
/// between its options. An application assembling radios out of separate widgets
/// gets three tab stops here, and that is the bug.
#[test]
fn tab_steps_over_the_whole_group_rather_than_through_it() {
    let (mut ui, before, group, after) = radios();
    ui.focus(Some(before));

    ui.handle(&[key(KeyCode::Tab)]);
    assert_eq!(ui.focused(), Some(group), "into the group");

    ui.handle(&[key(KeyCode::Tab)]);
    assert_eq!(
        ui.focused(),
        Some(after),
        "and straight out the other side — one stop for three options"
    );
    assert!(
        ui.messages().is_empty(),
        "tabbing past a group must not change what it has chosen"
    );
}

/// Arrows belong to the group only while it holds focus; a group that swallowed
/// them regardless would fight whatever else on the panel wants them.
#[test]
fn arrows_do_nothing_to_an_unfocused_group() {
    let (mut ui, before, group, _) = radios();
    ui.focus(Some(before));
    ui.handle(&[key(KeyCode::ArrowDown)]);
    assert_eq!(mode(&ui, group), 0);
    assert!(ui.messages().is_empty());
}

#[test]
fn a_disabled_group_neither_chooses_nor_takes_focus() {
    let (mut ui, _, group, _) = radios();
    ui.set_enabled(group, false);

    ui.handle(&click(60, 65));
    assert_eq!(mode(&ui, group), 0);
    assert!(ui.messages().is_empty());

    ui.focus(Some(group));
    assert_eq!(ui.focused(), None);
}

#[test]
fn setting_the_choice_from_the_application_emits_nothing() {
    let (mut ui, _, group, _) = radios();
    ui.widget_mut::<RadioGroup<Msg>>(group)
        .expect("group")
        .set_selected(2);
    assert_eq!(mode(&ui, group), 2);
    assert!(ui.messages().is_empty());
}

/// An empty group is not a tab stop. Focusing something no key does anything to
/// strands a keyboard-only panel on it.
#[test]
fn an_empty_group_is_skipped_by_tab() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let before = ui
        .add(
            root,
            Button::new("Before", Msg::Save),
            Rect::new(20, 200, 100, 30),
        )
        .expect("before");
    ui.add(
        root,
        RadioGroup::<Msg>::inert(Vec::<String>::new()),
        Rect::new(20, 20, 200, 90),
    )
    .expect("empty");
    let after = ui
        .add(
            root,
            Button::new("After", Msg::Cancel),
            Rect::new(140, 200, 100, 30),
        )
        .expect("after");

    ui.focus(Some(before));
    ui.handle(&[key(KeyCode::Tab)]);
    assert_eq!(ui.focused(), Some(after), "the empty group is not a stop");
}

/// The drawing: exactly one option is filled, and moving the choice moves which.
/// A group whose `selected` index changes while the picture does not is a control
/// nobody can read, and every test above would still pass.
///
/// Only the **circle** is compared, not the row. Rows carry different words, so
/// two rows can never be pixel-equal however identically their circles are drawn
/// — which is what makes the circle the thing to look at.
#[test]
fn exactly_one_option_is_drawn_as_chosen() {
    let (mut ui, _, group, _) = radios();
    let bounds = ui.bounds(group).expect("bounds");
    let side = theme::DARK.metrics.size_selector;

    // Row `i` spans 30px; the circle is `side` square, centred in it.
    let circle = |i: i32| Rect::new(bounds.x, bounds.y + i * 30 + (30 - side) / 2, side, side);
    let shot =
        |ui: &mut Ui<Msg>| -> Vec<Vec<u32>> { (0..3).map(|i| pixels_of(ui, circle(i))).collect() };

    let first = shot(&mut ui);
    assert_ne!(
        first[0], first[1],
        "the chosen circle should differ from the rest"
    );
    assert_eq!(
        first[1], first[2],
        "two unchosen circles are the same drawing, so the same pixels"
    );

    ui.widget_mut::<RadioGroup<Msg>>(group)
        .expect("group")
        .set_selected(1);
    let second = shot(&mut ui);

    assert_eq!(
        second[1], first[0],
        "the newly chosen circle should look exactly like the old chosen one"
    );
    assert_eq!(
        second[0], first[1],
        "and the one it replaced should look exactly like an unchosen one"
    );
    assert_eq!(second[0], second[2], "still only one is filled");
}

// ------------------------------------------------------------------- progress

/// A bar at 20,20 measuring 200x12, between two buttons so Tab has a path.
fn bar(value: f32) -> (Ui<Msg>, NodeId, NodeId, NodeId) {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let before = ui
        .add(
            root,
            Button::new("Before", Msg::Save),
            Rect::new(20, 100, 100, 30),
        )
        .expect("before");
    let id = ui
        .add(root, Progress::new(value), Rect::new(20, 20, 200, 12))
        .expect("bar");
    let after = ui
        .add(
            root,
            Button::new("After", Msg::Cancel),
            Rect::new(140, 100, 100, 30),
        )
        .expect("after");
    (ui, before, id, after)
}

/// It reports; it does not take input. A bar that could be tabbed to would strand
/// a keyboard-only panel on something no key does anything to.
#[test]
fn a_progress_bar_is_not_a_tab_stop_and_not_clickable() {
    let (mut ui, before, id, after) = bar(0.5);

    ui.focus(Some(before));
    ui.handle(&[key(KeyCode::Tab)]);
    assert_eq!(ui.focused(), Some(after), "Tab went straight past the bar");

    ui.handle(&click(120, 26));
    assert!(ui.messages().is_empty());
    assert_eq!(
        ui.focused(),
        None,
        "clicking it drops focus like the background"
    );

    ui.focus(Some(id));
    assert_eq!(
        ui.focused(),
        None,
        "and it cannot be focused directly either"
    );
}

/// The drawing. A `value` that changes while the picture does not is a bar that
/// reports nothing, and every unit test on the arithmetic would still pass.
#[test]
fn the_filled_portion_follows_the_value() {
    // The pixel at the midpoint of the track: track colour below half, fill
    // colour above it. One pixel is enough because it is the *right* pixel.
    let midpoint = |value: f32| -> u32 {
        let (mut ui, _, id, _) = bar(value);
        let bounds = ui.bounds(id).expect("bounds");
        let probe = Rect::new(
            bounds.x + bounds.width / 2,
            bounds.y + bounds.height / 2,
            1,
            1,
        );
        pixels_of(&mut ui, probe)[0]
    };

    let quarter = midpoint(0.25);
    let three_quarters = midpoint(0.75);
    assert_ne!(
        quarter, three_quarters,
        "the midpoint pixel should be track at 25% and fill at 75%"
    );
    assert_eq!(
        midpoint(0.0),
        quarter,
        "empty and a quarter agree at the midpoint"
    );
    assert_eq!(
        midpoint(1.0),
        three_quarters,
        "so do full and three quarters"
    );
}

/// The case the widget exists to survive: `done / total` with `total` at zero.
/// A panic here is a black screen on a kiosk, and a one-pixel bar is a lie.
#[test]
fn a_bar_fed_a_nan_draws_the_same_as_an_empty_one() {
    let empty = {
        let (mut ui, _, id, _) = bar(0.0);
        let bounds = ui.bounds(id).expect("bounds");
        pixels_of(&mut ui, bounds)
    };

    let (mut ui, _, id, _) = bar(0.5);
    let bounds = ui.bounds(id).expect("bounds");
    // The division a caller writes, not a folded `f32::NAN`: `done / total` on
    // the first frame, before anything has counted how much there is to do.
    let done = core::hint::black_box(0.0f32);
    let total = core::hint::black_box(0.0f32);
    ui.widget_mut::<Progress>(id)
        .expect("bar")
        .set_value(done / total);
    let nan = pixels_of(&mut ui, bounds);

    assert_eq!(nan, empty, "a NaN drew something other than an empty bar");
}

// --------------------------------------------------------------------- slider

fn down(x: i32, y: i32) -> InputEvent {
    InputEvent::PointerButton {
        button: PointerButton::Left,
        state: ElementState::Down,
        position: Point::new(x, y),
        modifiers: Modifiers::NONE,
    }
}

fn moved(x: i32, y: i32) -> InputEvent {
    InputEvent::PointerMoved {
        position: Point::new(x, y),
    }
}

fn up(x: i32, y: i32) -> InputEvent {
    InputEvent::PointerButton {
        button: PointerButton::Left,
        state: ElementState::Up,
        position: Point::new(x, y),
        modifiers: Modifiers::NONE,
    }
}

/// A 0..100 slider at 20,20 measuring 200x40, starting at 0.
fn slider() -> (Ui<Msg>, NodeId) {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let id = ui
        .add(
            root,
            Slider::new(0.0, 100.0, 0.0, Msg::Level),
            Rect::new(20, 20, 200, 40),
        )
        .expect("slider");
    (ui, id)
}

fn level(ui: &Ui<Msg>, id: NodeId) -> f32 {
    ui.widget::<Slider<Msg>>(id).expect("slider").value()
}

/// Pressing the track goes there rather than stepping towards it.
#[test]
fn pressing_the_track_jumps_to_that_value() {
    let (mut ui, id) = slider();
    ui.handle(&[down(120, 40), up(120, 40)]);
    assert!(
        (level(&ui, id) - 50.0).abs() < 5.0,
        "a press at the midpoint gave {}",
        level(&ui, id)
    );
    assert!(!ui.messages().is_empty(), "and reported it");
}

/// **The case this widget exists for.** A drag that leaves the rectangle keeps
/// control of the pointer: the tree routes moves to the pressed widget wherever
/// it goes, but it clears `PRESSED` at the boundary, so a slider reading its
/// drag state from the tree would stop tracking exactly here.
#[test]
fn a_drag_keeps_following_the_pointer_after_it_leaves_the_widget() {
    let (mut ui, id) = slider();

    ui.handle(&[down(30, 40)]);
    let started = level(&ui, id);

    // Out through the bottom of the widget, and keep going sideways well outside
    // it — the pointer is nowhere near the slider now.
    ui.handle(&[moved(60, 40), moved(120, 200), moved(180, 300)]);
    let dragged = level(&ui, id);
    assert!(
        dragged > started + 40.0,
        "the drag stopped tracking once the pointer left: {started} -> {dragged}"
    );

    ui.handle(&[up(180, 300)]);
    let released = level(&ui, id);

    // And once released, moving over the widget again must not move the value.
    ui.handle(&[moved(40, 40), moved(200, 40)]);
    assert_eq!(
        level(&ui, id),
        released,
        "the release did not end the drag; the knob is still following the pointer"
    );
}

/// Dragged past either end it clamps. Wrapping a volume from full to silent
/// because a finger slid too far is the failure this pins.
#[test]
fn dragging_past_either_end_clamps_rather_than_wrapping() {
    let (mut ui, id) = slider();

    ui.handle(&[down(120, 40), moved(100_000, 40)]);
    assert_eq!(level(&ui, id), 100.0);

    ui.handle(&[moved(-100_000, 40)]);
    assert_eq!(level(&ui, id), 0.0, "and back down the other way");

    ui.handle(&[up(-100_000, 40)]);
}

/// The keyboard contract: arrows step, pages step further, Home and End go to
/// the ends. A panel with no pointer has to be drivable by these alone.
#[test]
fn the_keyboard_moves_the_value_by_step_page_and_end() {
    let (mut ui, id) = slider();
    ui.focus(Some(id));

    ui.handle(&[key(KeyCode::ArrowRight)]);
    assert_eq!(level(&ui, id), 1.0, "one hundredth of the range");
    ui.handle(&[key(KeyCode::ArrowLeft)]);
    assert_eq!(level(&ui, id), 0.0);

    ui.handle(&[key(KeyCode::PageUp)]);
    assert_eq!(level(&ui, id), 10.0, "a tenth");

    ui.handle(&[key(KeyCode::End)]);
    assert_eq!(level(&ui, id), 100.0);
    ui.handle(&[key(KeyCode::Home)]);
    assert_eq!(level(&ui, id), 0.0);

    // At an end, a further press changes nothing and so reports nothing.
    ui.drain_messages().for_each(drop);
    ui.handle(&[key(KeyCode::ArrowLeft)]);
    assert_eq!(level(&ui, id), 0.0);
    assert!(
        ui.messages().is_empty(),
        "a value that did not move was reported"
    );
}

/// A step makes the value land on the grid however the pointer falls between.
#[test]
fn a_stepped_slider_snaps_wherever_the_pointer_lands() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let id = ui
        .add(
            root,
            Slider::new(0.0, 10.0, 0.0, Msg::Level).with_step(1.0),
            Rect::new(20, 20, 200, 40),
        )
        .expect("slider");

    for x in 20..220 {
        ui.handle(&[down(x, 40), up(x, 40)]);
        let value = level(&ui, id);
        assert_eq!(
            value,
            (value as i32) as f32,
            "x {x} gave {value}, which is not on the step grid"
        );
        assert!((0.0..=10.0).contains(&value), "x {x} gave {value}");
    }
}

#[test]
fn a_disabled_slider_neither_drags_nor_takes_focus() {
    let (mut ui, id) = slider();
    ui.set_enabled(id, false);

    ui.handle(&[down(120, 40), moved(180, 40), up(180, 40)]);
    assert_eq!(level(&ui, id), 0.0);
    assert!(ui.messages().is_empty());

    ui.focus(Some(id));
    assert_eq!(ui.focused(), None);
}

#[test]
fn setting_a_slider_from_the_application_emits_nothing() {
    let (mut ui, id) = slider();
    ui.widget_mut::<Slider<Msg>>(id)
        .expect("slider")
        .set_value(42.0);
    assert_eq!(level(&ui, id), 42.0);
    assert!(ui.messages().is_empty());
}

/// The drawing follows the value, and the knob is somewhere different at each
/// end. A slider whose value moves while the picture does not reports nothing.
#[test]
fn the_knob_moves_with_the_value() {
    let picture = |value: f32| -> Vec<u32> {
        let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
        let root = ui.root();
        let id = ui
            .add(
                root,
                Slider::new(0.0, 100.0, value, Msg::Level),
                Rect::new(20, 20, 200, 40),
            )
            .expect("slider");
        let bounds = ui.bounds(id).expect("bounds");
        pixels_of(&mut ui, bounds)
    };

    let (empty, half, full) = (picture(0.0), picture(50.0), picture(100.0));
    assert_ne!(empty, half);
    assert_ne!(half, full);
    assert_ne!(empty, full);
}

// -------------------------------------------------------------------- divider

/// A rule reports nothing and takes nothing. Tab must step straight over it, or
/// a keyboard-only panel strands on a line.
#[test]
fn a_divider_is_inert_and_not_a_tab_stop() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let before = ui
        .add(
            root,
            Button::new("Before", Msg::Save),
            Rect::new(20, 100, 100, 30),
        )
        .expect("before");
    let rule = ui
        .add(root, Divider::labelled("eller"), Rect::new(20, 20, 200, 24))
        .expect("divider");
    let after = ui
        .add(
            root,
            Button::new("After", Msg::Cancel),
            Rect::new(140, 100, 100, 30),
        )
        .expect("after");

    ui.focus(Some(before));
    ui.handle(&[key(KeyCode::Tab)]);
    assert_eq!(ui.focused(), Some(after), "Tab went straight past the rule");

    ui.handle(&click(120, 32));
    assert!(ui.messages().is_empty());
    ui.focus(Some(rule));
    assert_eq!(
        ui.focused(),
        None,
        "a rule cannot be focused directly either"
    );
}

/// The label breaks the rule, and the two are genuinely different pictures. A
/// divider that ignored its label would pass every geometry test in the module.
#[test]
fn a_label_breaks_the_rule_and_draws_between_the_halves() {
    let picture = |divider: Divider| -> Vec<u32> {
        let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
        let root = ui.root();
        let id = ui
            .add(root, divider, Rect::new(20, 20, 200, 24))
            .expect("divider");
        let bounds = ui.bounds(id).expect("bounds");
        pixels_of(&mut ui, bounds)
    };

    let plain = picture(Divider::new());
    let labelled = picture(Divider::labelled("eller"));
    assert_ne!(plain, labelled, "the label changed nothing on screen");

    // The rule sits at `(height - thickness) / 2`, which is not `height / 2` —
    // derived rather than guessed, because guessing it samples background and
    // then two identical rows of nothing compare equal and prove nothing.
    let thickness = theme::DARK.metrics.border.max(1) as usize;
    let row = (24 - thickness) / 2;
    let at = |x: usize| row * 200 + x;

    assert_ne!(
        plain[at(100)],
        labelled[at(100)],
        "the rule was not broken where the label sits"
    );
    assert_eq!(
        plain[at(2)],
        labelled[at(2)],
        "but the ends are still ruled"
    );
    assert_ne!(
        plain[at(2)],
        plain[at(2) - 200],
        "the sampled row is not the rule at all"
    );
}

/// A vertical divider is a different drawing, not a horizontal one in a narrow
/// box — and it ignores a label rather than drawing text across its own line.
#[test]
fn a_vertical_divider_rules_down_and_ignores_its_label() {
    let picture = |divider: Divider| -> Vec<u32> {
        let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
        let root = ui.root();
        let id = ui
            .add(root, divider, Rect::new(40, 20, 24, 120))
            .expect("divider");
        let bounds = ui.bounds(id).expect("bounds");
        pixels_of(&mut ui, bounds)
    };

    let bare = picture(Divider::vertical());
    let mut labelled_divider = Divider::vertical();
    labelled_divider.set_label("ignorert");
    let labelled = picture(labelled_divider);
    assert_eq!(bare, labelled, "a vertical rule drew its label");

    // A horizontal rule in the same box would paint one row across; a vertical
    // one paints one column down. Comparing them is the cheapest way to say so.
    let horizontal = picture(Divider::new());
    assert_ne!(
        bare, horizontal,
        "vertical and horizontal drew the same thing"
    );
}

// ---------------------------------------------------------------------- badge

/// A badge annotates; it does not take part. Tab must step over it, and a click
/// must fall through to whatever is underneath — a badge sitting on a row button
/// that swallowed the click would make the row unselectable.
#[test]
fn a_badge_is_inert_and_lets_clicks_through_to_what_is_under_it() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let before = ui
        .add(
            root,
            Button::new("Before", Msg::Save),
            Rect::new(20, 140, 100, 30),
        )
        .expect("before");
    // A row, with a badge sitting on top of it.
    let row = ui
        .add(
            root,
            Button::new("Row", Msg::Cancel),
            Rect::new(20, 20, 300, 40),
        )
        .expect("row");
    let badge = ui
        .add(row, Badge::new("3"), Rect::new(250, 8, 40, 24))
        .expect("badge");
    let after = ui
        .add(
            root,
            Button::new("After", Msg::Submitted),
            Rect::new(140, 140, 100, 30),
        )
        .expect("after");

    ui.focus(Some(before));
    ui.handle(&[key(KeyCode::Tab)]);
    assert_eq!(ui.focused(), Some(row), "the row is the next stop");
    ui.handle(&[key(KeyCode::Tab)]);
    assert_eq!(ui.focused(), Some(after), "and the badge is not one");

    // A click squarely on the badge reaches the row underneath.
    ui.handle(&click(290, 48));
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Msg::Cancel],
        "the badge swallowed a click meant for the row"
    );

    ui.focus(Some(badge));
    assert_eq!(
        ui.focused(),
        None,
        "a badge cannot be focused directly either"
    );
}

/// Text longer than the badge is clipped to the pill rather than running across
/// whatever is beside it. The tree clips the canvas to the widget's bounds, so
/// this is really a test that the badge relies on that instead of measuring and
/// truncating itself.
#[test]
fn text_too_long_for_a_badge_is_clipped_to_it() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let id = ui
        .add(
            root,
            Badge::new("VEDLIKEHOLDSMODUS"),
            Rect::new(40, 40, 40, 24),
        )
        .expect("badge");

    // A band across the badge's row, wider than the badge on both sides.
    let band = Rect::new(0, 40, 200, 24);
    let painted = pixels_of(&mut ui, band);
    let bounds = ui.bounds(id).expect("bounds");

    let background = painted[0];
    for y in 0..band.height {
        for x in 0..band.width {
            let inside = x >= bounds.x - band.x && x < bounds.right() - band.x;
            if !inside {
                assert_eq!(
                    painted[(y * band.width + x) as usize],
                    background,
                    "something was drawn at {x},{y}, outside the badge"
                );
            }
        }
    }
}

/// The role decides the colour, and two roles are two different pictures. A
/// badge that ignored its role would pass every sizing test in the module.
#[test]
fn the_role_changes_what_is_drawn() {
    let picture = |role: Role| -> Vec<u32> {
        let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
        let root = ui.root();
        let id = ui
            .add(
                root,
                Badge::new("PÅ").with_role(role),
                Rect::new(40, 40, 60, 24),
            )
            .expect("badge");
        let bounds = ui.bounds(id).expect("bounds");
        pixels_of(&mut ui, bounds)
    };
    assert_ne!(picture(Role::Primary), picture(Role::Error));
    assert_ne!(picture(Role::Success), picture(Role::Warning));
}

// ---------------------------------------------------------------------- alert

/// A banner reports; it does not take part. Tab steps over it and a click falls
/// through, the same as a badge.
#[test]
fn an_alert_is_inert_and_not_a_tab_stop() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let before = ui
        .add(
            root,
            Button::new("Before", Msg::Save),
            Rect::new(20, 160, 100, 30),
        )
        .expect("before");
    let alert = ui
        .add(
            root,
            Alert::new(Role::Error, "Kunne ikke lagre"),
            Rect::new(20, 20, 300, 40),
        )
        .expect("alert");
    let after = ui
        .add(
            root,
            Button::new("After", Msg::Cancel),
            Rect::new(140, 160, 100, 30),
        )
        .expect("after");

    ui.focus(Some(before));
    ui.handle(&[key(KeyCode::Tab)]);
    assert_eq!(
        ui.focused(),
        Some(after),
        "Tab went straight past the banner"
    );

    ui.handle(&click(120, 40));
    assert!(ui.messages().is_empty());
    ui.focus(Some(alert));
    assert_eq!(ui.focused(), None);
}

/// The message actually wraps onto more than one line, which is the half of this
/// widget that needed new machinery in `denise-text`.
///
/// Counted as inked rows against a one-word banner in the same box, rather than
/// as "bands of text". Bands looked right and were not: the sample strip caught
/// the rounded corners' antialiasing, so the test passed even with wrapping
/// removed. Two pictures of the same shape cancel that out.
#[test]
fn a_long_message_wraps_onto_more_than_one_line() {
    let inked_rows = |message: &str| -> usize {
        let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
        let root = ui.root();
        let id = ui
            .add(
                root,
                Alert::new(Role::Warning, message),
                Rect::new(20, 20, 200, 90),
            )
            .expect("alert");
        let bounds = ui.bounds(id).expect("bounds");
        let painted = pixels_of(&mut ui, bounds);

        // Inside the corner radius on both sides, so the rounded rect's own
        // antialiasing cannot be mistaken for ink.
        let inset = theme::DARK.radius(denise::Radius::Box) + 2;
        let strip = inset..(bounds.width - inset);
        let fill = painted[((bounds.height / 2) * bounds.width + bounds.width / 2) as usize];
        (0..bounds.height)
            .filter(|y| {
                strip
                    .clone()
                    .any(|x| painted[(y * bounds.width + x) as usize] != fill)
            })
            .count()
    };

    let one = inked_rows("Full");
    let many = inked_rows("Disken er nesten full og det er ikke plass til flere opptak");
    assert!(one > 0, "the short message drew nothing at all");
    assert!(
        many > one * 2,
        "a message that should wrap to several lines inked {many} rows against \
         {one} for a single word, so it did not wrap"
    );
}

/// The role decides both colours together — which is the whole reason this is a
/// widget rather than a `Panel` and a `Label` a caller pairs by hand.
#[test]
fn the_role_changes_the_banner_and_its_text_together() {
    let picture = |role: Role| -> Vec<u32> {
        let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
        let root = ui.root();
        let id = ui
            .add(root, Alert::new(role, "Lagret"), Rect::new(20, 20, 220, 40))
            .expect("alert");
        let bounds = ui.bounds(id).expect("bounds");
        pixels_of(&mut ui, bounds)
    };
    assert_ne!(picture(Role::Success), picture(Role::Error));
    assert_ne!(picture(Role::Info), picture(Role::Warning));
}

/// More message than banner is clipped to the banner rather than drawn across
/// whatever is below it.
#[test]
fn a_message_taller_than_its_banner_is_clipped_to_it() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let id = ui
        .add(
            root,
            Alert::new(
                Role::Error,
                "en to tre fire fem seks sju atte ni ti elleve tolv",
            ),
            Rect::new(40, 40, 120, 30),
        )
        .expect("alert");
    let bounds = ui.bounds(id).expect("bounds");

    let band = Rect::new(0, 40, 300, 60);
    let painted = pixels_of(&mut ui, band);
    let background = painted[0];
    for y in 0..band.height {
        for x in 0..band.width {
            let inside = x >= bounds.x - band.x && x < bounds.right() - band.x && y < bounds.height;
            if !inside {
                assert_eq!(
                    painted[(y * band.width + x) as usize],
                    background,
                    "the message escaped the banner at {x},{y}"
                );
            }
        }
    }
}

// ----------------------------------------------------------------------- tabs

/// A strip at 20,20, wide enough for three tabs, with buttons either side.
fn strip() -> (Ui<Msg>, NodeId, NodeId, NodeId) {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let before = ui
        .add(
            root,
            Button::new("Before", Msg::Save),
            Rect::new(20, 160, 100, 30),
        )
        .expect("before");
    let tabs = ui
        .add(
            root,
            Tabs::new(["En", "To", "Tre"], Msg::Page),
            Rect::new(20, 20, 360, 40),
        )
        .expect("tabs");
    let after = ui
        .add(
            root,
            Button::new("After", Msg::Cancel),
            Rect::new(140, 160, 100, 30),
        )
        .expect("after");
    (ui, before, tabs, after)
}

fn page(ui: &Ui<Msg>, id: NodeId) -> usize {
    ui.widget::<Tabs<Msg>>(id).expect("tabs").selected()
}

/// **The rule this widget shares with `RadioGroup`.** Tab moves from the strip
/// into the page, not through three tabs first.
#[test]
fn tab_steps_over_the_whole_strip_rather_than_through_it() {
    let (mut ui, before, tabs, after) = strip();
    ui.focus(Some(before));

    ui.handle(&[key(KeyCode::Tab)]);
    assert_eq!(ui.focused(), Some(tabs), "into the strip");
    ui.handle(&[key(KeyCode::Tab)]);
    assert_eq!(ui.focused(), Some(after), "and out the far side");
    assert!(
        ui.messages().is_empty(),
        "tabbing past must not change the page"
    );
}

/// Left and Right move and wrap; Home and End reach the ends.
#[test]
fn the_arrows_move_the_selection_and_wrap() {
    let (mut ui, _, tabs, _) = strip();
    ui.focus(Some(tabs));

    ui.handle(&[key(KeyCode::ArrowRight)]);
    assert_eq!(page(&ui, tabs), 1);
    ui.handle(&[key(KeyCode::ArrowRight), key(KeyCode::ArrowRight)]);
    assert_eq!(page(&ui, tabs), 0, "past the end wraps to the start");
    ui.handle(&[key(KeyCode::ArrowLeft)]);
    assert_eq!(page(&ui, tabs), 2, "and before the start wraps to the end");

    ui.handle(&[key(KeyCode::Home)]);
    assert_eq!(page(&ui, tabs), 0);
    ui.handle(&[key(KeyCode::End)]);
    assert_eq!(page(&ui, tabs), 2);

    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![
            Msg::Page(1),
            Msg::Page(2),
            Msg::Page(0),
            Msg::Page(2),
            Msg::Page(0),
            Msg::Page(2),
        ],
        "every move reported once, and no move reported nothing"
    );
}

/// Up and Down belong to the page below, not to the strip — the opposite of a
/// vertical radio group, which takes all four.
#[test]
fn up_and_down_are_left_for_the_page_below() {
    let (mut ui, _, tabs, _) = strip();
    ui.focus(Some(tabs));
    ui.handle(&[key(KeyCode::ArrowDown), key(KeyCode::ArrowUp)]);
    assert_eq!(page(&ui, tabs), 0);
    assert!(ui.messages().is_empty());
}

/// Clicking a tab selects it, and clicking the selected one again reports
/// nothing.
#[test]
fn clicking_a_tab_selects_it_and_reselecting_reports_nothing() {
    let (mut ui, _, tabs, _) = strip();

    ui.handle(&click(30, 40));
    assert_eq!(page(&ui, tabs), 0);
    assert!(ui.messages().is_empty(), "tab 0 was already selected");

    // Somewhere in the middle of the strip is not tab 0.
    ui.handle(&click(160, 40));
    assert_ne!(page(&ui, tabs), 0, "a click further along should move it");
    assert_eq!(ui.drain_messages().collect::<Vec<_>>().len(), 1);
}

#[test]
fn a_disabled_strip_neither_selects_nor_takes_focus() {
    let (mut ui, _, tabs, _) = strip();
    ui.set_enabled(tabs, false);

    ui.handle(&click(160, 40));
    assert_eq!(page(&ui, tabs), 0);
    assert!(ui.messages().is_empty());

    ui.focus(Some(tabs));
    assert_eq!(ui.focused(), None);
}

/// The underline moves with the selection, and lands under the selected tab at
/// each end of the row. A strip whose index changes while the picture does not
/// is a control that reports nothing.
#[test]
fn the_underline_lands_under_the_selected_tab_at_each_end() {
    let underline_span = |selected: usize| -> (i32, i32) {
        let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
        let root = ui.root();
        let id = ui
            .add(
                root,
                Tabs::new(["En", "To", "Tre"], Msg::Page).with_selected(selected),
                Rect::new(20, 20, 360, 40),
            )
            .expect("tabs");
        let bounds = ui.bounds(id).expect("bounds");
        let painted = pixels_of(&mut ui, bounds);

        // The bottom row is the rule, with the selected tab's segment in a
        // different colour. The rule is the row's *modal* colour — taking the
        // pixel at x=0 instead reads the selected segment itself when tab 0 is
        // the selected one, and then "differs from the rule" finds every tab but
        // that one.
        let row = bounds.height - 1;
        let mut counts = std::collections::HashMap::new();
        for x in 0..bounds.width {
            *counts
                .entry(painted[(row * bounds.width + x) as usize])
                .or_insert(0usize) += 1;
        }
        let rule = *counts.iter().max_by_key(|(_, n)| **n).expect("pixels").0;
        let marked: Vec<i32> = (0..bounds.width)
            .filter(|x| painted[(row * bounds.width + x) as usize] != rule)
            .collect();
        assert!(!marked.is_empty(), "no underline at all for tab {selected}");
        (marked[0], marked[marked.len() - 1])
    };

    let (first_start, first_end) = underline_span(0);
    let (last_start, last_end) = underline_span(2);

    assert_eq!(
        first_start, 0,
        "the first tab's underline starts at the edge"
    );
    assert!(
        last_start > first_end,
        "the last tab's underline ({last_start}..{last_end}) should be entirely \
         past the first tab's ({first_start}..{first_end})"
    );
}

// ----------------------------------------------------------------------- list

/// A four-row list at 20,20 with buttons either side, rows 40 high so the
/// arithmetic in the tests is obvious. Row 2 is disabled.
fn settings() -> (Ui<Msg>, NodeId, NodeId, NodeId) {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let before = ui
        .add(
            root,
            Button::new("Before", Msg::Save),
            Rect::new(280, 20, 100, 30),
        )
        .expect("before");
    let list = ui
        .add(
            root,
            List::new(
                vec![
                    ListItem::new("Nettverk"),
                    ListItem::new("Skjerm").with_trailing("70 %"),
                    ListItem::new("Avriming").disabled(),
                    ListItem::new("Om"),
                ],
                Msg::Row,
            )
            .on_activate(Msg::Open)
            .with_row_height(40),
            Rect::new(20, 20, 240, 160),
        )
        .expect("list");
    let after = ui
        .add(
            root,
            Button::new("After", Msg::Cancel),
            Rect::new(280, 60, 100, 30),
        )
        .expect("after");
    (ui, before, list, after)
}

fn row(ui: &Ui<Msg>, id: NodeId) -> Option<usize> {
    ui.widget::<List<Msg>>(id).expect("list").selected()
}

/// The centre of a row, in surface coordinates.
fn at_row(index: i32) -> (i32, i32) {
    (60, 20 + index * 40 + 20)
}

/// **The rule this widget shares with `RadioGroup` and `Tabs`.** Tab moves past
/// the whole list, not through four rows.
#[test]
fn tab_steps_over_the_whole_list_rather_than_through_it() {
    let (mut ui, before, list, after) = settings();
    ui.focus(Some(before));

    ui.handle(&[key(KeyCode::Tab)]);
    assert_eq!(ui.focused(), Some(list), "into the list");
    ui.handle(&[key(KeyCode::Tab)]);
    assert_eq!(ui.focused(), Some(after), "and out the far side");
    assert!(ui.messages().is_empty(), "tabbing past must select nothing");
}

/// **The difference from `RadioGroup` and `Tabs`.** A long list that jumped from
/// the bottom row to the top under a held key would be disorienting, so the ends
/// are ends.
#[test]
fn the_arrows_stop_at_the_ends_rather_than_wrapping() {
    let (mut ui, _, list, _) = settings();
    ui.focus(Some(list));
    assert_eq!(row(&ui, list), None, "a list opens with nothing chosen");

    // The first press lands on the near end rather than doing nothing visible.
    ui.handle(&[key(KeyCode::ArrowDown)]);
    assert_eq!(row(&ui, list), Some(0));

    ui.handle(&keys(KeyCode::ArrowDown, 6));
    assert_eq!(
        row(&ui, list),
        Some(3),
        "six presses past the end must stay on the last row"
    );
    ui.handle(&keys(KeyCode::ArrowUp, 6));
    assert_eq!(
        row(&ui, list),
        Some(0),
        "and six back must stay on the first"
    );

    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![
            Msg::Row(0),
            Msg::Row(1),
            Msg::Row(3),
            Msg::Row(1),
            Msg::Row(0),
        ],
        "every move reported once, the disabled row skipped, and the presses \
         that moved nothing reported nothing"
    );
}

/// Home and End reach the first and last rows anybody can choose — not the first
/// and last rows.
#[test]
fn home_and_end_reach_the_ends_that_can_be_selected() {
    let (mut ui, _, list, _) = settings();
    ui.focus(Some(list));

    ui.handle(&[key(KeyCode::End)]);
    assert_eq!(row(&ui, list), Some(3));
    ui.handle(&[key(KeyCode::Home)]);
    assert_eq!(row(&ui, list), Some(0));

    // With the last row disabled too, End stops one short of it.
    ui.widget_mut::<List<Msg>>(list)
        .expect("list")
        .set_row_enabled(3, false);
    ui.handle(&[key(KeyCode::End)]);
    assert_eq!(row(&ui, list), Some(1), "row 2 and row 3 are both out");
}

/// A click selects, and a click on a disabled row does nothing at all — not even
/// move the selection off where it was.
#[test]
fn clicking_selects_and_a_disabled_row_is_inert() {
    let (mut ui, _, list, _) = settings();

    let (x, y) = at_row(1);
    ui.handle(&click(x, y));
    assert_eq!(row(&ui, list), Some(1));
    assert_eq!(ui.drain_messages().collect::<Vec<_>>(), vec![Msg::Row(1)]);

    let (x, y) = at_row(2);
    ui.handle(&click(x, y));
    assert_eq!(
        row(&ui, list),
        Some(1),
        "a click on a disabled row must leave the selection where it was"
    );
    assert!(ui.messages().is_empty());

    // And the empty space below the last row is background, not the last row.
    ui.handle(&click(60, 20 + 4 * 40 + 5));
    assert_eq!(row(&ui, list), Some(1));
    assert!(ui.messages().is_empty());
}

/// Selecting a row and acting on it are two different things, so they are two
/// different messages. Two clicks inside the window are one of each — never two
/// selections.
#[test]
fn a_double_click_activates_and_a_slow_second_click_does_not() {
    let (mut ui, _, _, _) = settings();
    let (x, y) = at_row(0);

    ui.tick(1_000);
    ui.handle(&click(x, y));
    ui.tick(1_100);
    ui.handle(&click(x, y));
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Msg::Row(0), Msg::Open(0)],
        "one selection and one activation"
    );

    // Two clicks a second apart are two selections and nothing else — and the
    // second reports nothing, because the row was already selected.
    ui.tick(5_000);
    ui.handle(&click(x, y));
    ui.tick(9_000);
    ui.handle(&click(x, y));
    assert!(
        ui.messages().is_empty(),
        "a slow click on the selected row is not an activation: {:?}",
        ui.messages()
    );
}

/// Enter activates the selected row, so a panel with no pointer can drive this.
#[test]
fn enter_activates_the_selected_row_and_nothing_else_does() {
    let (mut ui, _, list, _) = settings();
    ui.focus(Some(list));

    // Nothing selected: Enter has nothing to act on.
    ui.handle(&[key(KeyCode::Enter)]);
    assert!(ui.messages().is_empty());

    ui.handle(&[key(KeyCode::ArrowDown)]);
    assert_eq!(ui.drain_messages().collect::<Vec<_>>(), vec![Msg::Row(0)]);

    ui.handle(&[key(KeyCode::Enter)]);
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Msg::Open(0)],
        "Enter activates without reselecting"
    );
    assert_eq!(row(&ui, list), Some(0));
}

/// A menu on a touch panel activates on one tap: a double-tap is unreliable on a
/// resistive screen and unexpected on any of them.
#[test]
fn a_single_click_list_activates_on_the_first_tap() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    ui.add(
        root,
        List::new(["En", "To"], Msg::Row)
            .on_activate(Msg::Open)
            .activate_on_click()
            .with_row_height(40),
        Rect::new(20, 20, 240, 80),
    )
    .expect("menu");

    ui.handle(&click(60, 40));
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Msg::Row(0), Msg::Open(0)]
    );
}

#[test]
fn a_disabled_list_neither_selects_nor_takes_focus() {
    let (mut ui, _, list, _) = settings();
    ui.set_enabled(list, false);

    let (x, y) = at_row(1);
    ui.handle(&click(x, y));
    assert_eq!(row(&ui, list), None);
    assert!(ui.messages().is_empty());

    ui.focus(Some(list));
    assert_eq!(ui.focused(), None);
}

/// The selected row has to *look* selected. A list whose index moves while the
/// picture does not is a control that reports nothing.
#[test]
fn the_selected_row_is_drawn_differently_from_the_rest() {
    // The modal colour of a strip across the row, so the sample is the row's
    // fill and not a glyph or the rounded corner's antialiasing.
    let fill_of = |ui: &mut Ui<Msg>, index: i32| -> u32 {
        let strip = Rect::new(20, 20 + index * 40 + 20, 240, 1);
        let painted = pixels_of(ui, strip);
        let mut counts = std::collections::HashMap::new();
        for pixel in painted {
            *counts.entry(pixel).or_insert(0usize) += 1;
        }
        *counts.iter().max_by_key(|(_, n)| **n).expect("pixels").0
    };

    let (mut ui, _, list, _) = settings();
    let resting = fill_of(&mut ui, 0);
    assert_eq!(resting, fill_of(&mut ui, 1), "nothing is selected yet");

    ui.handle(&click(at_row(1).0, at_row(1).1));
    assert_eq!(row(&ui, list), Some(1));
    assert_ne!(
        fill_of(&mut ui, 1),
        resting,
        "the selected row is drawn exactly like the others"
    );
    assert_eq!(fill_of(&mut ui, 0), resting, "and its neighbours are not");
}

/// Hover follows the pointer between rows, and goes out when the pointer leaves
/// the list.
///
/// The leaving is the part worth pinning: `PointerLeft` never reaches a widget,
/// and the tree clears `HOVERED` without telling it, so a list that trusted its
/// own memory would leave a row lit under a pointer somewhere else entirely.
#[test]
fn hover_follows_the_pointer_and_goes_out_when_it_leaves() {
    let fill_of = |ui: &mut Ui<Msg>, index: i32| -> u32 {
        let strip = Rect::new(20, 20 + index * 40 + 20, 240, 1);
        let painted = pixels_of(ui, strip);
        let mut counts = std::collections::HashMap::new();
        for pixel in painted {
            *counts.entry(pixel).or_insert(0usize) += 1;
        }
        *counts.iter().max_by_key(|(_, n)| **n).expect("pixels").0
    };

    let (mut ui, _, _, _) = settings();
    let resting = fill_of(&mut ui, 0);

    let (x, y) = at_row(0);
    ui.handle(&[InputEvent::PointerMoved {
        position: Point::new(x, y),
    }]);
    let lit = fill_of(&mut ui, 0);
    assert_ne!(lit, resting, "the row under the pointer is not lit");
    assert_eq!(fill_of(&mut ui, 1), resting, "and only that row is");

    let (x, y) = at_row(1);
    ui.handle(&[InputEvent::PointerMoved {
        position: Point::new(x, y),
    }]);
    assert_eq!(fill_of(&mut ui, 0), resting, "the old row stayed lit");
    assert_eq!(fill_of(&mut ui, 1), lit, "the new row did not light");

    // A disabled row is not a hover target either — it cannot be chosen, so
    // lighting it up under the pointer would be a lie.
    let (x, y) = at_row(2);
    ui.handle(&[InputEvent::PointerMoved {
        position: Point::new(x, y),
    }]);
    assert_eq!(fill_of(&mut ui, 2), resting, "a disabled row lit up");

    // Back onto a row that *does* light, so there is something to put out. A
    // disabled row leaves nothing remembered, and leaving from there would pass
    // this test whether or not the guard exists.
    let (x, y) = at_row(3);
    ui.handle(&[InputEvent::PointerMoved {
        position: Point::new(x, y),
    }]);
    assert_eq!(
        fill_of(&mut ui, 3),
        lit,
        "row 3 should be lit before leaving"
    );

    ui.handle(&[InputEvent::PointerLeft]);
    for index in 0..4 {
        assert_eq!(
            fill_of(&mut ui, index),
            resting,
            "row {index} stayed lit after the pointer left the surface"
        );
    }
}

// ------------------------------------------------------------------ animation

/// The widget #19 said was impossible: appears, waits, fades, and is never
/// focused at any point. Written against `Widget` here in the tests, exactly the
/// way an application would.
struct Toast {
    born_ms: Option<u64>,
    lifetime_ms: u64,
}

impl Toast {
    fn new(lifetime_ms: u64) -> Self {
        Self {
            born_ms: None,
            lifetime_ms,
        }
    }
}

impl Widget<Msg> for Toast {
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut denise_render::Canvas<'_>) {
        // The fade is time-driven, which is the whole point of the test.
        let age = self
            .born_ms
            .map_or(0, |born| ctx.now_ms.saturating_sub(born));
        let alpha = 255u64.saturating_sub(age * 255 / self.lifetime_ms.max(1)) as u8;
        canvas.fill_rect(ctx.bounds, denise::Color::rgba(200, 200, 200, alpha));
    }

    fn animate(&mut self, now_ms: u64) -> Animation {
        let born = *self.born_ms.get_or_insert(now_ms);
        if now_ms.saturating_sub(born) >= self.lifetime_ms {
            // Expired. Answering `None` is the hand-back; the application
            // notices through `animating()` falling, or just removes the node.
            return Animation::NONE;
        }
        Animation {
            repaint: true,
            next_ms: Some(now_ms + 16),
        }
    }
}

/// A toast animates from birth to expiry without ever being focused, and a tree
/// at rest afterwards asks for nothing. This test is the issue's "done when"
/// list, verbatim.
#[test]
fn a_toast_fades_without_ever_being_focused() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let toast = ui
        .add(root, Toast::new(300), Rect::new(100, 180, 200, 40))
        .expect("toast");

    // Nothing is focused, and nothing ever will be.
    assert_eq!(ui.focused(), None);
    ui.request_animation(toast);
    assert!(
        ui.next_wake_ms().is_some(),
        "requesting animation wakes the loop even before the first tick"
    );

    ui.tick(0);
    assert_eq!(ui.animating(), 1);
    ui.render_nothing();

    ui.tick(100);
    assert!(ui.needs_paint(), "a fading toast owes a frame");
    assert_eq!(ui.focused(), None, "and still nothing is focused");
    ui.render_nothing();

    // Expiry: the toast stops asking, and the tree is back at rest.
    ui.tick(300);
    ui.tick(316);
    assert_eq!(ui.animating(), 0, "an expired toast must stop asking");
    assert_eq!(ui.next_wake_ms(), None, "the loop may sleep indefinitely");
}

/// Two widgets animate at once — the caret and a toast — and the scene wakes
/// for the more impatient of the two. One stopping leaves the other running.
#[test]
fn two_widgets_animate_at_once_and_the_scene_wakes_for_the_sooner() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let field = ui
        .add(root, TextInput::<Msg>::new(), Rect::new(20, 20, 200, 40))
        .expect("field");
    let toast = ui
        .add(root, Toast::new(200), Rect::new(100, 180, 200, 40))
        .expect("toast");

    ui.focus(Some(field));
    ui.request_animation(toast);
    ui.tick(0);
    assert_eq!(ui.animating(), 2, "the caret and the toast, concurrently");

    // The toast wants a frame in 16 ms; the caret's blink edge is at 500 ms.
    let wake = ui.next_wake_ms().expect("two animations pending");
    assert_eq!(wake, 16, "the scene wakes for the more impatient animation");

    // The toast expires; the caret keeps blinking, undisturbed.
    ui.tick(250);
    assert_eq!(ui.animating(), 1, "the toast is done, the caret is not");
    let wake = ui.next_wake_ms().expect("the caret still blinks");
    assert!(wake >= 500, "the survivor's cadence, not the departed's");
}

/// Hiding a node stops its animation: an invisible spinner spinning forever is
/// the exact failure the animating set exists to make visible. Removal likewise.
#[test]
fn hiding_or_removing_an_animating_node_stops_its_animation() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    // Unbounded on purpose: this one would never stop asking on its own.
    let spinner = ui
        .add(root, Toast::new(u64::MAX), Rect::new(20, 20, 40, 40))
        .expect("spinner");
    ui.request_animation(spinner);
    ui.tick(0);
    assert_eq!(ui.animating(), 1);

    ui.set_visible(spinner, false);
    assert_eq!(ui.animating(), 0, "a hidden node must not hold the CPU");
    ui.tick(16);
    assert_eq!(ui.next_wake_ms(), None);

    // And re-showing does not quietly resume: animation is requested, not
    // remembered.
    ui.set_visible(spinner, true);
    ui.tick(32);
    assert_eq!(
        ui.animating(),
        0,
        "re-showing must not resurrect the request"
    );

    ui.request_animation(spinner);
    ui.tick(48);
    assert_eq!(ui.animating(), 1);
    ui.remove(spinner);
    assert_eq!(
        ui.animating(),
        0,
        "removal stops the animation with the node"
    );
}

/// The README's idle number, as an assertion: a fully populated panel at rest
/// — widgets of every kind, nothing focused, nothing mid-transition — requests
/// no wakes and holds nothing in the animating set.
#[test]
fn a_tree_at_rest_requests_no_wakes() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    ui.add(root, Panel::default(), Rect::new(10, 10, 380, 220))
        .expect("panel");
    ui.add(root, Label::new("Temp"), Rect::new(20, 20, 100, 20))
        .expect("label");
    ui.add(
        root,
        Button::new("Lagre", Msg::Save),
        Rect::new(20, 44, 100, 30),
    )
    .expect("button");
    ui.add(root, TextInput::<Msg>::new(), Rect::new(20, 80, 200, 30))
        .expect("field");
    let toggle = ui
        .add(
            root,
            Toggle::new("Mute", Msg::Muted),
            Rect::new(20, 116, 160, 24),
        )
        .expect("toggle");
    ui.add(root, Progress::new(0.4), Rect::new(20, 146, 200, 10))
        .expect("bar");

    ui.tick(0);
    assert_eq!(ui.animating(), 0, "nothing has any business animating");
    assert_eq!(ui.next_wake_ms(), None, "the loop may block indefinitely");

    // A toggle click starts a bounded transition; its end returns to rest.
    ui.handle(&click(30, 128));
    let _ = toggle;
    ui.tick(0);
    assert_eq!(ui.animating(), 1, "the crossing is a bounded exception");
    ui.tick(1_000);
    // The caret of nothing: the click focused the toggle, not the field, and a
    // toggle at rest does not blink. Everything must be back to zero.
    assert_eq!(ui.animating(), 0, "the transition ended and handed back");
    assert_eq!(ui.next_wake_ms(), None, "at rest again");
}

// ------------------------------------------------------------------- popups

use denise_ui::Side;

/// A base scene with a button, and a popup anchored to it holding another
/// button — the dropdown shape, without pretending to be a dropdown widget.
fn popped() -> (Ui<Msg>, NodeId, NodeId, NodeId) {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let opener = ui
        .add(
            root,
            Button::new("Velg", Msg::Save),
            Rect::new(40, 40, 120, 36),
        )
        .expect("opener");
    let container = ui
        .push_popup(opener, Size::new(160, 90), Side::Below)
        .expect("popup");
    ui.add(container, Panel::default(), Rect::new(0, 0, 160, 90))
        .expect("panel");
    let choice = ui
        .add(
            container,
            Button::new("Alternativ", Msg::Cancel),
            Rect::new(8, 8, 144, 30),
        )
        .expect("choice");
    (ui, opener, container, choice)
}

/// The container sits below its anchor with the documented gap, and the caller
/// owns its content — the Tabs decision, again.
#[test]
fn a_popup_is_anchored_below_its_opener() {
    let (ui, opener, container, _) = popped();
    let anchor = ui.bounds(opener).expect("anchor");
    let popup = ui.bounds(container).expect("container");
    assert_eq!(popup.y, anchor.bottom() + 4);
    assert_eq!(popup.x, anchor.x);
}

/// Content inside the popup receives input normally: a press on the choice is
/// a press on the choice.
#[test]
fn content_inside_a_popup_works_normally() {
    let (mut ui, _, container, _) = popped();
    let bounds = ui.bounds(container).expect("container");
    ui.handle(&click(bounds.x + 20, bounds.y + 20));
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Msg::Cancel],
        "the choice button should have fired"
    );
}

/// **The classic bug, pinned.** A press outside the popup closes it and is
/// swallowed — it must not also reach whatever is underneath. The opener sits
/// exactly under the press to make the failure loud.
#[test]
fn the_dismissing_press_is_swallowed_not_delivered() {
    let (mut ui, opener, container, _) = popped();
    let target = ui.bounds(opener).expect("opener");

    // Press dead on the opener, which is underneath the (closed-over) base
    // scene while the popup is up.
    ui.handle(&click(target.x + 10, target.y + 10));
    assert!(
        !ui.contains(container),
        "the outside press should have closed the popup"
    );
    assert!(
        ui.messages().is_empty(),
        "and nothing underneath may fire: {:?}",
        ui.messages()
    );

    // The *next* press is an ordinary press again and reaches the opener.
    ui.handle(&click(target.x + 10, target.y + 10));
    assert_eq!(ui.drain_messages().collect::<Vec<_>>(), vec![Msg::Save]);
}

/// Escape closes the popup and focus returns to the anchor — a keyboard user
/// is standing exactly where they were before it opened.
#[test]
fn escape_closes_the_popup_and_focus_returns_to_the_anchor() {
    let (mut ui, opener, container, choice) = popped();
    ui.focus(Some(choice));
    assert_eq!(ui.focused(), Some(choice));

    ui.handle(&[key(KeyCode::Escape)]);
    assert!(!ui.contains(container), "Escape should close the popup");
    assert_eq!(
        ui.focused(),
        Some(opener),
        "focus should return to the anchor"
    );
}

/// Dismissing by pointer returns focus to the anchor too — the rule holds
/// however the popup closes, including a plain `close_popup` call.
#[test]
fn focus_returns_to_the_anchor_however_the_popup_closes() {
    let (mut ui, opener, _, _) = popped();
    ui.handle(&click(300, 300));
    assert_eq!(ui.focused(), Some(opener), "after light dismiss");

    let (mut ui, opener, _, _) = popped();
    assert!(ui.close_popup());
    assert_eq!(ui.focused(), Some(opener), "after close_popup");
    assert!(!ui.close_popup(), "no popup left to close");
}

/// Tab is confined to the popup while it is up — the same structural trap a
/// modal gets, because input only ever reaches the topmost scene.
#[test]
fn tab_is_confined_inside_the_popup() {
    let (mut ui, opener, _, choice) = popped();
    ui.handle(&[key(KeyCode::Tab), key(KeyCode::Tab), key(KeyCode::Tab)]);
    assert_eq!(
        ui.focused(),
        Some(choice),
        "however many tabs, focus stays in the popup"
    );
    let _ = opener;
}

/// A popup near the bottom edge opens upwards: the flip, driven through the
/// real tree rather than the geometry function alone.
#[test]
fn a_popup_near_the_bottom_edge_opens_upwards() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let opener = ui
        .add(
            root,
            Button::new("Velg", Msg::Save),
            Rect::new(40, 200, 120, 36),
        )
        .expect("opener near the bottom of a 240-tall surface");
    let container = ui
        .push_popup(opener, Size::new(160, 90), Side::Below)
        .expect("popup");
    let anchor = ui.bounds(opener).expect("anchor");
    let popup = ui.bounds(container).expect("container");
    assert_eq!(popup.bottom(), anchor.y - 4, "flipped above");
}

/// A popup inside a modal is ordinary nesting: the popup captures input while
/// it is up, closing it returns input to the modal, and the modal's dim is not
/// doubled by the popup's scene.
#[test]
fn a_popup_inside_a_modal_nests_and_does_not_double_dim() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    ui.add(root, Panel::default(), Rect::new(0, 0, 400, 240))
        .expect("page");

    let modal = ui.push_scene(110);
    let opener = ui
        .add(
            modal,
            Button::new("Velg", Msg::Save),
            Rect::new(120, 60, 120, 36),
        )
        .expect("opener in the modal");

    // The dimmed backdrop as it looks with just the modal.
    let sample = Rect::new(10, 220, 40, 12);
    let with_modal = pixels_of(&mut ui, sample);

    let container = ui
        .push_popup(opener, Size::new(140, 60), Side::Below)
        .expect("popup over a modal");
    ui.add(container, Panel::default(), Rect::new(0, 0, 140, 60))
        .expect("panel");
    let with_popup = pixels_of(&mut ui, sample);
    assert_eq!(
        with_modal, with_popup,
        "the popup must not darken what the modal already dimmed"
    );

    // Closing the popup returns input to the modal, not to the base scene.
    ui.handle(&click(20, 225));
    assert!(!ui.contains(container), "closed");
    ui.handle(&click(180, 78));
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Msg::Save],
        "the modal's opener is reachable again"
    );
}

/// Two dimmed scenes do not stack their veils: the backdrop under both is as
/// dark as one veil, not two.
#[test]
fn two_modals_do_not_double_dim_the_backdrop() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    ui.add(root, Panel::default(), Rect::new(0, 0, 400, 240))
        .expect("page");
    let sample = Rect::new(10, 220, 40, 12);

    ui.push_scene(110);
    let one_veil = pixels_of(&mut ui, sample);
    ui.push_scene(110);
    let two_scenes = pixels_of(&mut ui, sample);
    assert_eq!(
        one_veil, two_scenes,
        "a second dimmed scene must not darken the base again"
    );
}

/// A popup whose anchor's scene it belongs to conceptually — pops in the wrong
/// order still leave a sane stack. `pop_scene` pops the popup first because it
/// is topmost; there is no way to pop the modal out from under it, which is
/// the property worth asserting.
#[test]
fn pops_in_the_wrong_order_cannot_strand_a_popup() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let modal = ui.push_scene(110);
    let opener = ui
        .add(
            modal,
            Button::new("Velg", Msg::Save),
            Rect::new(40, 40, 120, 36),
        )
        .expect("opener");
    let container = ui
        .push_popup(opener, Size::new(100, 50), Side::Below)
        .expect("popup");

    // The application tries to close the modal while the popup is up. What
    // actually pops is the topmost scene — the popup — which is the only
    // answer that leaves no scene referring to dead nodes.
    assert!(ui.pop_scene());
    assert!(!ui.contains(container), "the popup went first");
    assert!(ui.contains(opener), "the modal survived");
    assert!(ui.pop_scene(), "the modal pops normally afterwards");
    assert!(!ui.pop_scene(), "and the base scene never pops");
}

// ----------------------------------------------------------------- scrolling

/// A 120-tall viewport over 360 of content: three buttons at 0, 120 and 240,
/// each 40 tall, inside a scrollable panel.
fn viewport() -> (Ui<Msg>, NodeId, NodeId, NodeId, NodeId) {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let view = ui
        .add(root, Panel::default(), Rect::new(40, 40, 200, 120))
        .expect("viewport");
    ui.set_scrollable(view, true);
    let top = ui
        .add(
            view,
            Button::new("Top", Msg::Save),
            Rect::new(10, 0, 180, 40),
        )
        .expect("top");
    let middle = ui
        .add(
            view,
            Button::new("Middle", Msg::Cancel),
            Rect::new(10, 120, 180, 40),
        )
        .expect("middle");
    let bottom = ui
        .add(
            view,
            Button::new("Bottom", Msg::Submitted),
            Rect::new(10, 240, 180, 40),
        )
        .expect("bottom");
    (ui, view, top, middle, bottom)
}

/// Paint, clip and hit testing agree about a scrolled child, because one reflow
/// computes all three. Scrolled down by one page, the bottom button is where
/// the top one was — visually and to the pointer.
#[test]
fn paint_and_hit_testing_agree_about_a_scrolled_child() {
    let (mut ui, view, top, _, bottom) = viewport();

    // Before scrolling: the top button is visible and clickable at y=50.
    ui.handle(&click(100, 60));
    assert_eq!(ui.drain_messages().collect::<Vec<_>>(), vec![Msg::Save]);
    assert!(
        ui.bounds(bottom).expect("bottom").y > 160,
        "bottom starts below the fold"
    );

    // Asking for more than the content has clamps to the end: 280 of content
    // in a 120 viewport scrolls at most 160.
    ui.set_scroll(view, Point::new(0, 240));
    assert_eq!(
        ui.bounds(bottom).expect("bottom").y,
        120,
        "bounds moved with the (clamped) scroll"
    );
    ui.handle(&click(100, 140));
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Msg::Submitted],
        "the click lands on what is visually there"
    );
    // And the top button is scrolled out: clipped, unhittable, a click through
    // its old spot reaches nothing.
    ui.handle(&click(100, 60));
    assert!(
        ui.messages().is_empty(),
        "the scrolled-out button must not be clickable"
    );
    let _ = top;
}

/// Content outside the viewport is clipped out of painting entirely.
#[test]
fn content_outside_the_viewport_is_clipped() {
    let (mut ui, view, _, _, bottom) = viewport();
    let outside = Rect::new(40, 170, 200, 60);
    let before = pixels_of(&mut ui, outside);

    // Everything below the viewport's bottom edge (y=160) is background: the
    // 240-tall content must not leak past the 120-tall viewport.
    let mut reference: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let r = reference.root();
    reference
        .add(r, Panel::default(), Rect::new(40, 40, 200, 120))
        .expect("bare panel");
    let bare = pixels_of(&mut reference, outside);
    assert_eq!(before, bare, "content leaked out of the viewport");

    ui.set_scroll(view, Point::new(0, 999));
    let _ = bottom;
    let after = pixels_of(&mut ui, outside);
    assert_eq!(after, bare, "scrolling must not change what leaks: nothing");
}

/// The offset clamps to the content: no negative scroll, no scrolling past the
/// last child, and the getter reports what was actually applied.
#[test]
fn the_scroll_offset_clamps_to_the_content() {
    let (mut ui, view, _, _, _) = viewport();
    assert_eq!(ui.max_scroll(view), Point::new(0, 160), "280 tall in 120");

    ui.set_scroll(view, Point::new(-50, -50));
    assert_eq!(ui.scroll(view), Point::ZERO);
    ui.set_scroll(view, Point::new(500, 9_999));
    assert_eq!(ui.scroll(view), Point::new(0, 160));
}

/// The wheel scrolls the viewport under the pointer, without anything focused
/// and without the pointer resting on any widget.
#[test]
fn the_wheel_scrolls_the_viewport_under_the_pointer() {
    let (mut ui, view, _, _, _) = viewport();
    ui.handle(&[InputEvent::PointerScroll {
        delta_x: 0.0,
        delta_y: 48.0,
        position: Point::new(100, 100),
    }]);
    assert_eq!(ui.scroll(view), Point::new(0, 48), "content scrolled down");

    // And a wheel outside the viewport does nothing to it.
    ui.handle(&[InputEvent::PointerScroll {
        delta_x: 0.0,
        delta_y: 48.0,
        position: Point::new(350, 220),
    }]);
    assert_eq!(ui.scroll(view), Point::new(0, 48));
}

/// Tab to a widget below the fold scrolls it into view — a keyboard-only panel
/// must never focus something nobody can see.
#[test]
fn focusing_below_the_fold_scrolls_the_target_into_view() {
    let (mut ui, view, _, _, bottom) = viewport();
    ui.focus(Some(bottom));
    let bounds = ui.bounds(bottom).expect("bottom");
    let viewport = ui.bounds(view).expect("view");
    assert!(
        bounds.y >= viewport.y && bounds.bottom() <= viewport.bottom(),
        "the focused button must be inside the viewport: {bounds:?} in {viewport:?}"
    );
    assert!(ui.scroll(view).y > 0, "which required scrolling");
}

/// PageDown pages the scrollable that contains the focus, by its own height,
/// and PageUp pages back.
#[test]
fn the_page_keys_page_the_scrollable_holding_focus() {
    let (mut ui, view, top, _, _) = viewport();
    ui.focus(Some(top));
    ui.handle(&[key(KeyCode::PageDown)]);
    assert_eq!(ui.scroll(view), Point::new(0, 120), "one viewport height");
    ui.handle(&[key(KeyCode::PageDown)]);
    assert_eq!(ui.scroll(view), Point::new(0, 160), "clamped at the end");
    ui.handle(&[key(KeyCode::PageUp)]);
    assert_eq!(ui.scroll(view), Point::new(0, 40));
}

/// A touch on the viewport's background drags the content; a touch on a widget
/// belongs to the widget.
#[test]
fn a_touch_on_the_background_drags_the_scroll() {
    let (mut ui, view, _, _, _) = viewport();
    // y=90 is between the top button (ends 80) and the middle (starts 160):
    // background.
    ui.handle(&[
        InputEvent::TouchDown {
            id: 1,
            position: Point::new(100, 90),
        },
        InputEvent::TouchMoved {
            id: 1,
            position: Point::new(100, 50),
        },
        InputEvent::TouchUp {
            id: 1,
            position: Point::new(100, 50),
            cancelled: false,
        },
    ]);
    assert_eq!(
        ui.scroll(view),
        Point::new(0, 40),
        "dragging the finger up scrolls the content down"
    );
    assert!(ui.messages().is_empty(), "and no widget fired");

    // A touch that lands on a button is the button's press, not a drag.
    ui.set_scroll(view, Point::new(0, 0));
    ui.handle(&[
        InputEvent::TouchDown {
            id: 2,
            position: Point::new(100, 60),
        },
        InputEvent::TouchMoved {
            id: 2,
            position: Point::new(100, 55),
        },
        InputEvent::TouchUp {
            id: 2,
            position: Point::new(100, 55),
            cancelled: false,
        },
    ]);
    assert_eq!(ui.scroll(view), Point::ZERO, "the button kept its touch");
    assert_eq!(ui.drain_messages().collect::<Vec<_>>(), vec![Msg::Save]);
}

/// Scrolling damages the viewport: the next paint repaints it, and an idle
/// tree afterwards owes nothing.
#[test]
fn scrolling_damages_the_viewport() {
    let (mut ui, view, _, _, _) = viewport();
    ui.render_nothing();
    assert!(!ui.needs_paint());
    ui.set_scroll(view, Point::new(0, 30));
    assert!(ui.needs_paint(), "a scroll owes a frame");
    ui.render_nothing();
    ui.set_scroll(view, Point::new(0, 30));
    assert!(!ui.needs_paint(), "the same offset again owes nothing");
}

/// The reason the foundation exists: a list taller than its viewport, whose
/// keyboard selection walks below the fold and pulls the viewport along.
#[test]
fn a_list_selection_below_the_fold_pulls_the_viewport_along() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let view = ui
        .add(root, Panel::default(), Rect::new(20, 20, 240, 120))
        .expect("viewport");
    ui.set_scrollable(view, true);
    let list = ui
        .add(
            view,
            List::new(
                (1..=8).map(|i| format!("Rad {i}")).collect::<Vec<_>>(),
                Msg::Row,
            )
            .with_row_height(40),
            Rect::new(0, 0, 240, 320),
        )
        .expect("list");
    ui.focus(Some(list));

    // Walk the selection to the last row: 8 rows of 40 in a 120 viewport.
    ui.handle(&keys(KeyCode::ArrowDown, 8));
    let selected = ui
        .widget::<List<Msg>>(list)
        .expect("list")
        .selected()
        .expect("a selection");
    assert_eq!(selected, 7);

    let viewport = ui.bounds(view).expect("view");
    let list_bounds = ui.bounds(list).expect("list");
    let row = Rect::new(list_bounds.x, list_bounds.y + 7 * 40, list_bounds.width, 40);
    assert!(
        row.y >= viewport.y && row.bottom() <= viewport.bottom(),
        "the selected row must be inside the viewport: {row:?} in {viewport:?}"
    );
    assert_eq!(ui.scroll(view).y, 200, "scrolled to hold the last row");

    // And walking back up pulls it back.
    ui.handle(&keys(KeyCode::ArrowUp, 7));
    assert_eq!(ui.scroll(view).y, 0, "the first row is visible again");
}

/// The hovered widget sees the wheel before the tree does: one that consumes
/// it — a future value-spinner, a chart — stops the viewport underneath from
/// moving.
#[test]
fn a_widget_that_consumes_the_wheel_stops_the_viewport_scrolling() {
    struct WheelEater;
    impl Widget<Msg> for WheelEater {
        fn paint(&self, _: &mut PaintCtx<'_>, _: &mut denise_render::Canvas<'_>) {}
        fn on_event(
            &mut self,
            event: &denise_ui::Event<'_>,
            _: &mut denise_ui::EventCtx<'_, Msg>,
        ) -> denise_ui::Handled {
            match event {
                denise_ui::Event::Input(InputEvent::PointerScroll { .. }) => {
                    denise_ui::Handled::Yes
                }
                _ => denise_ui::Handled::No,
            }
        }
        fn accepts_pointer(&self) -> bool {
            true
        }
    }

    let (mut ui, view, _, _, _) = viewport();
    ui.add(view, WheelEater, Rect::new(0, 40, 200, 40))
        .expect("eater");
    let wheel = |x: i32, y: i32| InputEvent::PointerScroll {
        delta_x: 0.0,
        delta_y: 48.0,
        position: Point::new(x, y),
    };

    // Over the eater (absolute y 80..120): consumed, no scroll.
    ui.handle(&[wheel(100, 100)]);
    assert_eq!(ui.scroll(view), Point::ZERO, "the widget ate the wheel");

    // Over plain viewport (y 40..80 is the Top button, which does not consume;
    // y 130 is background): the tree scrolls.
    ui.handle(&[wheel(100, 130)]);
    assert_eq!(ui.scroll(view), Point::new(0, 48));
}

// ----------------------------------------------------------- radial progress

/// Renders a ring at each value and reports the painted-pixel count in the
/// ring's band, which is what "how much of it is filled" means in pixels.
#[cfg(test)]
fn ring_pixels(value: f32) -> usize {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let id = ui
        .add(
            root,
            RadialProgress::new(value).with_role(Role::Primary),
            Rect::new(40, 40, 120, 120),
        )
        .expect("ring");
    let bounds = ui.bounds(id).expect("bounds");
    let painted = pixels_of(&mut ui, bounds);
    // The value arc is Primary; the track is Base300. Count what is neither
    // background nor track by taking the least common colour's population —
    // simplest honest measure: pixels matching the Primary fill.
    let primary = theme::DARK.color(Role::Primary).to_argb8888();
    painted.iter().filter(|&&px| px == primary).count()
}

/// The ring fills with the value, and both ends are exact: nothing at zero, a
/// closed ring at one.
#[test]
fn the_ring_fills_with_its_value() {
    let empty = ring_pixels(0.0);
    let quarter = ring_pixels(0.25);
    let half = ring_pixels(0.5);
    let full = ring_pixels(1.0);

    assert_eq!(empty, 0, "an empty ring draws no arc at all");
    assert!(quarter > 0, "a quarter draws something");
    assert!(half > quarter, "{half} is not more than {quarter}");
    assert!(full > half, "{full} is not more than {half}");

    // A full ring is about four times a quarter — the arc length is linear in
    // the sweep, so a wildly different ratio means the geometry is wrong.
    let ratio = full as f32 / quarter as f32;
    assert!(
        (3.0..5.0).contains(&ratio),
        "full/quarter is {ratio}, so the sweep is not linear in the value"
    );
}

/// A full ring is closed all the way round — the property that lets a caller
/// pass `done / total` at 100% with no special case, since a sweep of a whole
/// turn *is* the circle.
///
/// Asserted by walking the ring's own band rather than by diffing against a
/// differently-coloured reference: at the faint end of the antialiasing a
/// `Base300` pixel rounds back to the background where a `Primary` one does
/// not, so a shape comparison across two colours compares the palette as much
/// as the geometry.
#[test]
fn a_full_ring_is_closed_all_the_way_round() {
    let sample = |value: f32| -> Vec<bool> {
        let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
        let root = ui.root();
        let id = ui
            .add(
                root,
                RadialProgress::new(value),
                Rect::new(40, 40, 120, 120),
            )
            .expect("ring");
        let bounds = ui.bounds(id).expect("bounds");
        let painted = pixels_of(&mut ui, bounds);
        let primary = theme::DARK.color(Role::Primary).to_argb8888();

        // Mid-band of the ring: radius 60, thickness 60/5 = 12, so the band is
        // 48..60 and its middle is 54.
        let (cx, cy) = (bounds.width / 2, bounds.height / 2);
        (0..48)
            .map(|step| {
                let angle = step as f32 / 48.0 * std::f32::consts::TAU;
                let x = cx + (54.0 * angle.sin()) as i32;
                let y = cy - (54.0 * angle.cos()) as i32;
                painted[(y * bounds.width + x) as usize] == primary
            })
            .collect()
    };

    let full = sample(1.0);
    assert!(
        full.iter().all(|&lit| lit),
        "a full ring has gaps in it: {} of {} samples unpainted",
        full.iter().filter(|&&lit| !lit).count(),
        full.len()
    );

    // And a half ring is lit for about half of the way round, starting at the
    // top and going clockwise.
    let half = sample(0.5);
    let lit = half.iter().filter(|&&lit| lit).count();
    assert!(
        (20..28).contains(&lit),
        "a half ring lit {lit} of 48 samples"
    );
    assert!(half[0], "which starts at twelve o'clock");
    assert!(half[12], "and runs clockwise through three");
    assert!(!half[36], "and not through nine");
}

/// The ring stays inside its rectangle, and inscribes rather than stretching
/// when the rectangle is not square.
#[test]
fn the_ring_stays_inside_a_non_square_rectangle() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let id = ui
        .add(root, RadialProgress::new(1.0), Rect::new(40, 60, 200, 80))
        .expect("ring");
    let bounds = ui.bounds(id).expect("bounds");

    // A band around the widget: nothing may be painted outside its rectangle.
    let band = Rect::new(20, 40, 240, 120);
    let painted = pixels_of(&mut ui, band);
    let background = painted[0];
    for y in 0..band.height {
        for x in 0..band.width {
            let inside = bounds.contains(Point::new(band.x + x, band.y + y));
            if !inside {
                assert_eq!(
                    painted[(y * band.width + x) as usize],
                    background,
                    "the ring escaped its bounds at {x},{y}"
                );
            }
        }
    }

    // And it is a circle of the short side: the corners of the wide rectangle
    // are untouched.
    let corners = pixels_of(&mut ui, Rect::new(bounds.x, bounds.y, 8, 8));
    assert!(
        corners.iter().all(|&p| p == background),
        "a wide rectangle stretched the ring into its corner"
    );
}

/// A label of a sensible size lands in the hole and never touches the ring
/// band. What this pins is the *centring*, not a clip: the widget deliberately
/// lets an oversized label overflow onto the ring rather than truncating a
/// number, so this asserts the case a caller should be in, not a guarantee the
/// widget makes for every string.
#[test]
fn a_sensible_label_is_drawn_in_the_hole_and_never_on_the_ring() {
    let render = |label: &str| {
        let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
        let root = ui.root();
        let id = ui
            .add(
                root,
                RadialProgress::new(0.7).with_label(label),
                Rect::new(40, 40, 120, 120),
            )
            .expect("ring");
        let bounds = ui.bounds(id).expect("bounds");
        (pixels_of(&mut ui, bounds), bounds)
    };
    let (bare, bounds) = render("");
    let (labelled, _) = render("70 %");
    assert_ne!(bare, labelled, "the label drew nothing");

    // Radius 60, thickness 12: the ring occupies 48..60 from the centre, so
    // every pixel the label added must be closer in than 48.
    let hole = 60 - 12;
    let (cx, cy) = (bounds.width / 2, bounds.height / 2);
    let mut changed = 0;
    for y in 0..bounds.height {
        for x in 0..bounds.width {
            let i = (y * bounds.width + x) as usize;
            if bare[i] == labelled[i] {
                continue;
            }
            changed += 1;
            let (dx, dy) = ((x - cx) as f32, (y - cy) as f32);
            let distance = (dx * dx + dy * dy).sqrt();
            assert!(
                distance < hole as f32,
                "the label reached the ring band at {x},{y}: {distance} from the centre"
            );
        }
    }
    assert!(
        changed > 20,
        "only {changed} pixels changed; is it drawing?"
    );
}

#[test]
fn a_radial_progress_is_inert_and_not_a_tab_stop() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let before = ui
        .add(
            root,
            Button::new("Before", Msg::Save),
            Rect::new(20, 200, 80, 30),
        )
        .expect("before");
    let ring = ui
        .add(root, RadialProgress::new(0.5), Rect::new(40, 40, 120, 120))
        .expect("ring");
    ui.focus(Some(before));
    ui.handle(&[key(KeyCode::Tab)]);
    assert_ne!(ui.focused(), Some(ring), "a ring must not take focus");
    ui.handle(&click(100, 100));
    assert!(ui.messages().is_empty(), "and must not swallow a click");
}

// ------------------------------------------------------------------- spinner

/// A spinner in a tree: added, started, spinning.
fn spinning() -> (Ui<Msg>, NodeId) {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let id = ui
        .add(root, Spinner::new(), Rect::new(60, 60, 64, 64))
        .expect("spinner");
    ui.request_animation(id);
    (ui, id)
}

/// It does not start itself. Adding a spinner to a tree keeps the device
/// asleep; asking it to animate is what costs, and the asking is explicit.
#[test]
fn a_spinner_does_not_start_itself() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let id = ui
        .add(root, Spinner::new(), Rect::new(60, 60, 64, 64))
        .expect("spinner");
    ui.tick(0);
    assert_eq!(ui.animating(), 0, "an unasked spinner must not spin");
    assert_eq!(ui.next_wake_ms(), None, "and must not keep the loop awake");

    ui.request_animation(id);
    ui.tick(0);
    assert_eq!(ui.animating(), 1);
    assert!(ui.next_wake_ms().is_some());
}

/// It never stops on its own — the unbounded case #19 made expressible, spent
/// here on purpose. A hundred frames in and it is still asking.
#[test]
fn a_spinner_keeps_asking_for_frames_indefinitely() {
    let (mut ui, _) = spinning();
    for frame in 0..100u64 {
        ui.tick(frame * 50);
        assert_eq!(ui.animating(), 1, "frame {frame}: it stopped");
    }
    assert!(ui.next_wake_ms().is_some(), "and it is still asking");
}

/// Hiding it is how you stop paying. This is the claim the whole idle story
/// rests on for the one widget able to break it.
#[test]
fn hiding_a_spinner_puts_the_tree_back_to_sleep() {
    let (mut ui, id) = spinning();
    ui.tick(0);
    assert_eq!(ui.animating(), 1);

    ui.set_visible(id, false);
    ui.tick(50);
    assert_eq!(ui.animating(), 0, "a hidden spinner must not spin");
    assert_eq!(
        ui.next_wake_ms(),
        None,
        "and the loop may block indefinitely again"
    );

    // Showing it again does not resume by itself: animation is requested, not
    // remembered.
    ui.set_visible(id, true);
    ui.tick(100);
    assert_eq!(
        ui.animating(),
        0,
        "re-showing must not resurrect the request"
    );
    ui.request_animation(id);
    ui.tick(150);
    assert_eq!(ui.animating(), 1);

    // And removing it stops it with the node.
    ui.remove(id);
    ui.tick(200);
    assert_eq!(ui.animating(), 0);
    assert_eq!(ui.next_wake_ms(), None);
}

/// The arc actually moves, and it is the arc that moves rather than the ring:
/// the pixels change between frames, and a spinner and a full ring of the same
/// size occupy the same band.
#[test]
fn the_arc_moves_between_frames() {
    let (mut ui, id) = spinning();
    let bounds = ui.bounds(id).expect("bounds");

    ui.tick(0);
    let first = pixels_of(&mut ui, bounds);
    // A quarter of a one-second revolution.
    ui.tick(250);
    let later = pixels_of(&mut ui, bounds);
    assert_ne!(first, later, "the arc did not move");

    // A full revolution later it is back where it started.
    ui.tick(1_250);
    let round_again = pixels_of(&mut ui, bounds);
    assert_eq!(round_again, later, "a full revolution must close the loop");
}

/// A spinning spinner owes a frame; one asked twice at the same instant does
/// not. The tree wakes for the most impatient animation and asks everybody, so
/// being asked early is routine and must not cost a repaint.
#[test]
fn a_frame_that_moved_nothing_owes_no_repaint() {
    let (mut ui, _) = spinning();
    ui.tick(0);
    ui.render_nothing();

    ui.tick(0);
    assert!(!ui.needs_paint(), "no time passed, so nothing to repaint");

    ui.tick(50);
    assert!(ui.needs_paint(), "a frame later the arc has moved");
}

#[test]
fn a_spinner_is_inert_and_not_a_tab_stop() {
    let (mut ui, id) = spinning();
    let before = ui
        .add(
            ui.root(),
            Button::new("Before", Msg::Save),
            Rect::new(20, 200, 80, 30),
        )
        .expect("before");
    ui.focus(Some(before));
    ui.handle(&[key(KeyCode::Tab)]);
    assert_ne!(ui.focused(), Some(id), "a spinner must not take focus");
    ui.handle(&click(92, 92));
    assert!(ui.messages().is_empty(), "and must not swallow a click");
}

// ------------------------------------------------------------------ tooltips

/// A button with a tooltip, and one without.
fn tipped() -> (Ui<Msg>, NodeId, NodeId) {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let save = ui
        .add(
            root,
            Button::new("Lagre", Msg::Save),
            Rect::new(40, 40, 100, 32),
        )
        .expect("save");
    let plain = ui
        .add(
            root,
            Button::new("Avbryt", Msg::Cancel),
            Rect::new(40, 100, 100, 32),
        )
        .expect("plain");
    ui.set_tooltip(save, "Lagrer endringene");
    (ui, save, plain)
}

fn hover(x: i32, y: i32) -> [InputEvent; 1] {
    [InputEvent::PointerMoved {
        position: Point::new(x, y),
    }]
}

/// **The coupling the feature stands on.** A kiosk blocks on input until the
/// tree says it wants waking, so resting the pointer must produce a deadline —
/// otherwise the bubble appears the next time something unrelated happens.
#[test]
fn resting_on_a_tooltip_asks_the_loop_to_wake_for_it() {
    let (mut ui, _, _) = tipped();
    ui.tick(0);
    assert_eq!(ui.next_wake_ms(), None, "an idle tree wakes for nothing");

    ui.handle(&hover(80, 55));
    ui.tick(0);
    let due = ui
        .next_wake_ms()
        .expect("resting on a tooltip must ask for a wake");
    assert!(due > 0, "and at a time in the future");

    // Nothing before the deadline; the bubble at it.
    ui.render_nothing();
    ui.tick(due - 1);
    assert!(!ui.needs_paint(), "the bubble appeared early");
    ui.tick(due);
    assert!(ui.needs_paint(), "the bubble did not appear");
    assert_eq!(
        ui.next_wake_ms(),
        None,
        "a shown bubble wants no further wakes"
    );
}

/// A widget without a tooltip asks for nothing — or every widget on the panel
/// would keep the loop awake.
#[test]
fn hovering_something_without_a_tooltip_wakes_nothing() {
    let (mut ui, _, _) = tipped();
    ui.handle(&hover(80, 115));
    ui.tick(0);
    assert_eq!(ui.next_wake_ms(), None);
    ui.render_nothing();
    ui.tick(10_000);
    assert!(!ui.needs_paint(), "something appeared that should not have");
}

/// The bubble is drawn, above the widgets, and goes away again when the pointer
/// moves on — leaving the screen exactly as it was.
#[test]
fn the_bubble_is_drawn_and_then_cleanly_removed() {
    let (mut ui, _, _) = tipped();
    let area = Rect::new(20, 20, 260, 140);

    ui.tick(0);
    let before = pixels_of(&mut ui, area);

    ui.handle(&hover(80, 55));
    ui.tick(0);
    let due = ui.next_wake_ms().expect("a deadline");
    ui.tick(due);
    let shown = pixels_of(&mut ui, area);
    assert_ne!(before, shown, "the bubble drew nothing");

    // Moving to the other button takes it away. The hovered button looks
    // different, so compare a strip the buttons do not occupy.
    let strip = Rect::new(20, 74, 260, 24);
    let bare_strip = {
        let mut fresh: Ui<Msg> = Ui::new(SIZE, theme::DARK);
        let root = fresh.root();
        fresh
            .add(
                root,
                Button::new("Lagre", Msg::Save),
                Rect::new(40, 40, 100, 32),
            )
            .expect("save");
        fresh
            .add(
                root,
                Button::new("Avbryt", Msg::Cancel),
                Rect::new(40, 100, 100, 32),
            )
            .expect("plain");
        pixels_of(&mut fresh, strip)
    };
    let with_bubble = pixels_of(&mut ui, strip);
    assert_ne!(with_bubble, bare_strip, "the bubble is not in the gap");

    ui.handle(&hover(80, 115));
    ui.tick(due);
    assert_eq!(
        pixels_of(&mut ui, strip),
        bare_strip,
        "the bubble left something behind when it went"
    );
}

/// A press or a key means the person moved on, and takes the bubble with it.
#[test]
fn a_press_or_a_key_dismisses_the_bubble() {
    let strip = Rect::new(20, 74, 260, 24);
    let shown_then = |after: &[InputEvent]| -> bool {
        let (mut ui, _, _) = tipped();
        ui.handle(&hover(80, 55));
        ui.tick(0);
        let due = ui.next_wake_ms().expect("a deadline");
        ui.tick(due);
        let shown = pixels_of(&mut ui, strip);
        ui.handle(after);
        ui.tick(due);
        pixels_of(&mut ui, strip) == shown
    };
    assert!(!shown_then(&click(80, 55)), "a press must dismiss it");
    assert!(!shown_then(&[key(KeyCode::Tab)]), "a key must dismiss it");
    assert!(
        !shown_then(&[InputEvent::PointerLeft]),
        "the pointer leaving must dismiss it"
    );
    // And a bare move within the same widget does not — moving is how somebody
    // arrives at the thing they are resting on.
    assert!(
        shown_then(&hover(82, 56)),
        "a small move must not dismiss it"
    );
}

/// The bubble is not a node: it cannot be hit, focused or tabbed to, and the
/// widget underneath keeps working while it is up.
#[test]
fn the_bubble_is_not_a_node() {
    let (mut ui, save, _) = tipped();
    ui.handle(&hover(80, 55));
    ui.tick(0);
    let due = ui.next_wake_ms().expect("a deadline");
    ui.tick(due);

    // The bubble sits just below the button; a press there hits whatever the
    // tree has, never the bubble.
    assert_eq!(ui.hit_test(Point::new(80, 80)), None, "the bubble was hit");
    // And the anchor still works.
    ui.handle(&click(80, 55));
    assert_eq!(ui.drain_messages().collect::<Vec<_>>(), vec![Msg::Save]);
    let _ = save;
}

/// Clearing a tooltip while the pointer rests on it shows nothing.
#[test]
fn a_tooltip_cleared_mid_wait_never_appears() {
    let (mut ui, save, _) = tipped();
    let strip = Rect::new(20, 74, 260, 24);
    ui.handle(&hover(80, 55));
    ui.tick(0);
    let due = ui.next_wake_ms().expect("a deadline");
    let before = pixels_of(&mut ui, strip);

    ui.clear_tooltip(save);
    ui.tick(due);
    assert_eq!(pixels_of(&mut ui, strip), before, "it appeared anyway");
    assert_eq!(ui.next_wake_ms(), None);
}

/// **The part only hardware would have caught.** A bubble that goes must
/// *damage* what it covered, or the pixels stay on a display that repaints only
/// what it was told to.
///
/// The pixel tests above cannot see this: they paint into a fresh buffer with
/// an undefined age, which is a full repaint, so a missing damage rectangle
/// looks perfect. A mutation removing the damage passed every one of them.
#[test]
fn a_bubble_that_goes_damages_what_it_covered() {
    let (mut ui, _, _) = tipped();
    // Settle first: a freshly built tree has the whole surface damaged, which
    // would swallow the rectangle this test is about.
    ui.render_nothing();

    ui.handle(&hover(80, 55));
    ui.tick(0);
    let due = ui.next_wake_ms().expect("a deadline");
    ui.render_nothing();
    ui.tick(due);

    let bubble = ui.paint_for_damage();
    assert!(
        bubble.iter().any(|r| r.y > 72 && r.y < 100),
        "the bubble appearing did not damage the gap below the button: {bubble:?}"
    );

    // Move to the other button. The buttons' own hover damage is up at y 40 and
    // down at y 100; only the bubble can account for the gap between them.
    ui.handle(&hover(80, 115));
    ui.tick(due);
    let gone = ui.paint_for_damage();
    let gap = Rect::new(40, 76, 100, 20);
    assert!(
        gone.iter().any(|r| r.intersects(&gap)),
        "the bubble left without repainting what it covered: {gone:?}"
    );
}

// -------------------------------------------------------------------- select

/// A select and something to tab to, so focus movement is observable.
fn selecting() -> (Ui<Msg>, NodeId, NodeId) {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let select = ui
        .add(
            root,
            Select::new(["Auto", "Manuell", "Av"], Msg::Save).with_placeholder("Velg modus"),
            Rect::new(40, 40, 180, 34),
        )
        .expect("select");
    let after = ui
        .add(
            root,
            Button::new("Etter", Msg::Cancel),
            Rect::new(40, 180, 100, 30),
        )
        .expect("after");
    (ui, select, after)
}

fn chosen(ui: &Ui<Msg>, id: NodeId) -> Option<usize> {
    ui.widget::<Select<Msg>>(id).expect("select").selected()
}

/// A click asks to be opened. The widget does not open anything itself — it
/// cannot, and that is the design.
#[test]
fn clicking_a_select_asks_to_be_opened() {
    let (mut ui, _, _) = selecting();
    ui.handle(&click(100, 55));
    assert_eq!(ui.drain_messages().collect::<Vec<_>>(), vec![Msg::Save]);
    assert!(!ui.close_popup(), "nothing opened by itself");
}

/// Enter, Space and Down open it. Left and Right must not: a closed select that
/// edits its own value as somebody tabs past it is the classic accidental-edit
/// bug.
#[test]
fn the_keys_that_open_it_and_the_ones_that_must_not() {
    for code in [KeyCode::Enter, KeyCode::Space, KeyCode::ArrowDown] {
        let (mut ui, select, _) = selecting();
        ui.focus(Some(select));
        ui.handle(&[key(code)]);
        assert_eq!(
            ui.drain_messages().collect::<Vec<_>>(),
            vec![Msg::Save],
            "{code:?} should open it"
        );
    }
    for code in [KeyCode::ArrowLeft, KeyCode::ArrowRight, KeyCode::ArrowUp] {
        let (mut ui, select, _) = selecting();
        ui.focus(Some(select));
        ui.handle(&[key(code)]);
        assert!(
            ui.messages().is_empty(),
            "{code:?} must not open it, and must not edit it"
        );
        assert_eq!(chosen(&ui, select), None, "{code:?} changed the value");
    }
}

/// The whole dropdown, end to end: open, choose with the keyboard, the popup
/// closes, focus comes back to the select, and the value took.
#[test]
fn a_dropdown_opens_chooses_and_closes() {
    let (mut ui, select, _) = selecting();
    ui.focus(Some(select));
    ui.handle(&[key(KeyCode::Enter)]);
    assert_eq!(ui.drain_messages().collect::<Vec<_>>(), vec![Msg::Save]);

    let popup = denise_ui::widgets::open_select(&mut ui, select, Msg::Row).expect("opened");
    assert!(ui.contains(popup), "the list is a popup scene");
    assert!(ui.bounds(popup).expect("popup").y > 70, "below the control");

    // The list opens focused, so the keyboard works immediately — and moving
    // the highlight reports nothing. A dropdown that emitted every row the
    // arrows passed over would have the application applying three values on
    // the way to the fourth.
    ui.handle(&[key(KeyCode::ArrowDown), key(KeyCode::ArrowDown)]);
    assert!(
        ui.messages().is_empty(),
        "moving the highlight is not choosing: {:?}",
        ui.messages()
    );

    // Enter chooses; the application closes and applies.
    ui.handle(&[key(KeyCode::Enter)]);
    assert_eq!(ui.drain_messages().collect::<Vec<_>>(), vec![Msg::Row(1)]);
    ui.close_popup();
    ui.widget_mut::<Select<Msg>>(select)
        .expect("select")
        .set_selected(Some(1));

    assert!(!ui.contains(popup), "the popup closed");
    assert_eq!(ui.focused(), Some(select), "focus came back to the control");
    assert_eq!(chosen(&ui, select), Some(1));
}

/// The open list lines up with the control it dropped out of, and is at least
/// as wide as its widest option.
#[test]
fn the_open_list_lines_up_with_the_control() {
    let (mut ui, select, _) = selecting();
    let anchor = ui.bounds(select).expect("anchor");
    let popup = denise_ui::widgets::open_select(&mut ui, select, Msg::Row).expect("opened");
    let bounds = ui.bounds(popup).expect("popup");
    assert_eq!(
        bounds.x, anchor.x,
        "the list is not aligned with the control"
    );
    assert!(
        bounds.width >= anchor.width,
        "the list is narrower than the control: {bounds:?} vs {anchor:?}"
    );
}

/// The open list is seeded with the current choice, so opening a select that
/// already has a value starts on it rather than on the first row.
#[test]
fn the_open_list_starts_on_the_current_choice() {
    let (mut ui, select, _) = selecting();
    ui.widget_mut::<Select<Msg>>(select)
        .expect("select")
        .set_selected(Some(2));
    denise_ui::widgets::open_select(&mut ui, select, Msg::Row).expect("opened");

    // The list took focus; Enter activates whatever it starts on.
    ui.handle(&[key(KeyCode::Enter)]);
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Msg::Row(2)],
        "the list did not start on the current choice"
    );
}

/// Escape closes without choosing, and focus returns to the select — #18's
/// popup contract, reached through the dropdown.
#[test]
fn escape_closes_the_dropdown_without_choosing() {
    let (mut ui, select, _) = selecting();
    let popup = denise_ui::widgets::open_select(&mut ui, select, Msg::Row).expect("opened");
    ui.handle(&[key(KeyCode::Escape)]);
    assert!(!ui.contains(popup), "Escape should close it");
    assert_eq!(ui.focused(), Some(select));
    assert_eq!(chosen(&ui, select), None, "and choose nothing");
}

/// A press outside closes it and is swallowed: the button underneath must not
/// also fire. The classic dropdown bug, reached through the real widget.
#[test]
fn a_press_outside_the_dropdown_does_not_reach_what_is_under_it() {
    let (mut ui, select, after) = selecting();
    let popup = denise_ui::widgets::open_select(&mut ui, select, Msg::Row).expect("opened");
    let target = ui.bounds(after).expect("after");

    ui.handle(&click(target.x + 10, target.y + 10));
    assert!(!ui.contains(popup), "the press should have closed it");
    assert!(
        ui.messages().is_empty(),
        "and must not reach the button under it: {:?}",
        ui.messages()
    );
}

/// A select with no options opens nothing and is not a tab stop.
#[test]
fn an_empty_select_opens_nothing() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let select = ui
        .add(
            root,
            Select::<Msg>::new(Vec::<String>::new(), Msg::Save),
            Rect::new(40, 40, 180, 34),
        )
        .expect("select");
    ui.handle(&click(100, 55));
    assert!(ui.messages().is_empty());
    assert!(denise_ui::widgets::open_select(&mut ui, select, Msg::Row).is_none());
    assert!(!ui.close_popup(), "and pushed no scene while failing");
}

// -------------------------------------------------------------------- toasts

/// A panel with a button, so a toast has something to cover.
fn toasting() -> (Ui<Msg>, NodeId) {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    // Near the bottom, where the toasts stack.
    let button = ui
        .add(
            root,
            Button::new("Under", Msg::Save),
            Rect::new(140, 190, 120, 34),
        )
        .expect("button");
    (ui, button)
}

/// The whole life without an application touching it, and the tree back at rest
/// afterwards.
#[test]
fn a_toast_appears_holds_and_goes_by_itself() {
    let (mut ui, _) = toasting();
    ui.tick(0);
    assert_eq!(ui.toasts(), 0);
    assert_eq!(ui.next_wake_ms(), None, "an idle tree wakes for nothing");

    ui.toast("Lagret", Role::Success);
    assert_eq!(ui.toasts(), 1);
    assert!(ui.needs_paint(), "it owes a frame immediately");

    // It is still there through the hold, and gone after it.
    ui.tick(1_000);
    assert_eq!(ui.toasts(), 1, "still holding");
    ui.tick(10_000);
    assert_eq!(ui.toasts(), 0, "gone without anybody removing it");
    assert_eq!(ui.next_wake_ms(), None, "and the loop may sleep again");
}

/// **The cost claim.** A holding toast asks for one wake, at the instant it
/// starts fading — not a frame cadence for four seconds. This is the whole
/// reason a toast is affordable on a device that is supposed to idle.
#[test]
fn a_holding_toast_asks_for_one_wake_not_a_frame_rate() {
    let (mut ui, _) = toasting();
    ui.toast("Lagret", Role::Success);

    // Through the fade-in it wants frames, which are close together.
    ui.tick(0);
    let soon = ui.next_wake_ms().expect("fading in");
    assert!(
        soon <= 100,
        "a fade should ask for a frame soon, got {soon}"
    );

    // Once it has arrived, the next wake is the whole hold away.
    ui.tick(200);
    let due = ui.next_wake_ms().expect("holding");
    assert!(
        due >= 3_000,
        "a holding toast should wake once, at the fade — got {due}"
    );

    // And nothing is owed in between.
    ui.render_nothing();
    ui.tick(due - 500);
    assert!(!ui.needs_paint(), "a holding toast repainted for nothing");
}

/// Two toasts stack rather than landing on each other, and the newest is
/// nearest the edge.
#[test]
fn two_toasts_stack_without_covering_each_other() {
    let (mut ui, _) = toasting();
    ui.toast("Først", Role::Info);
    ui.toast("Så dette", Role::Success);
    ui.tick(200);
    assert_eq!(ui.toasts(), 2);

    // Both drew: the painted area differs from one toast alone.
    let area = Rect::new(0, 120, 400, 120);
    let two = pixels_of(&mut ui, area);
    ui.clear_toasts();
    ui.toast("Så dette", Role::Success);
    ui.tick(200);
    let one = pixels_of(&mut ui, area);
    assert_ne!(two, one, "the second toast did not stack above the first");
}

/// **The dropdown bug in a new hat.** A press on a toast dismisses it and must
/// not reach the button it was covering.
#[test]
fn a_press_on_a_toast_does_not_reach_what_is_under_it() {
    let (mut ui, button) = toasting();
    ui.toast("Lagret", Role::Success);
    ui.tick(200);

    // The toast stacks up from the bottom edge, over the button.
    let target = ui.bounds(button).expect("button");
    let _ = target;
    let mut engine_probe = None;
    for y in (150..235).rev() {
        // Find a row the toast occupies by dismissing at it and seeing if the
        // press was consumed; restore afterwards.
        let mut probe: Ui<Msg> = Ui::new(SIZE, theme::DARK);
        let root = probe.root();
        probe
            .add(
                root,
                Button::new("Under", Msg::Save),
                Rect::new(140, 190, 120, 34),
            )
            .expect("button");
        probe.toast("Lagret", Role::Success);
        probe.tick(200);
        probe.handle(&click(200, y));
        if probe.toasts() == 0 {
            engine_probe = Some(y);
            break;
        }
    }
    let inside = engine_probe.expect("the toast must cover some row near the bottom");

    ui.handle(&click(200, inside));
    assert_eq!(ui.toasts(), 0, "the press should dismiss it");
    assert!(
        ui.messages().is_empty(),
        "and must not reach what is underneath: {:?}",
        ui.messages()
    );
}

/// A press somewhere else is not the toasts' business, and still reaches the
/// widget it was aimed at.
#[test]
fn a_press_away_from_a_toast_reaches_the_widget() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    ui.add(
        root,
        Button::new("Oppe", Msg::Save),
        Rect::new(40, 40, 120, 34),
    )
    .expect("button");
    ui.toast("Lagret", Role::Success);
    ui.tick(200);

    ui.handle(&click(80, 55));
    assert_eq!(ui.drain_messages().collect::<Vec<_>>(), vec![Msg::Save]);
    assert_eq!(ui.toasts(), 1, "and the toast is untouched");
}

/// The stack is capped: a panel that showed a backlog would be unreadable
/// exactly when something was going wrong.
#[test]
fn the_stack_is_capped() {
    let (mut ui, _) = toasting();
    for i in 0..8 {
        ui.toast(format!("Melding {i}"), Role::Info);
    }
    assert!(ui.toasts() <= 3, "{} toasts on screen", ui.toasts());
}

/// A toast damages what it covers when it goes — the failure the tooltip's
/// damage test exposed, checked here too because a full repaint hides it.
#[test]
fn a_toast_damages_what_it_covered_when_it_goes() {
    let (mut ui, _) = toasting();
    ui.toast("Lagret", Role::Success);
    ui.tick(200);
    ui.render_nothing();

    // Expire it. The damage must cover the strip it occupied near the bottom.
    ui.tick(10_000);
    let damage = ui.paint_for_damage();
    assert_eq!(ui.toasts(), 0);
    let strip = Rect::new(100, 180, 200, 50);
    assert!(
        damage.iter().any(|r| r.intersects(&strip)),
        "the toast left without repainting what it covered: {damage:?}"
    );
}

/// Clearing takes them all, read or not, and damages what they covered.
#[test]
fn clearing_takes_every_toast() {
    let (mut ui, _) = toasting();
    ui.toast("Ein", Role::Info);
    ui.toast("To", Role::Info);
    ui.tick(200);
    ui.render_nothing();

    ui.clear_toasts();
    assert_eq!(ui.toasts(), 0);
    assert!(ui.needs_paint(), "clearing must repaint what they covered");
}
