//! The tree's structural guarantees, and the one that subsumes all of them: that
//! a damage-driven repaint produces exactly the pixels a full repaint would.
//!
//! That property is the reason the tree owns invalidation rather than the
//! application. Every test below that pokes at the tree and then calls
//! [`assert_matches_full_repaint`] is really asking the same question — did the
//! tree damage everything that changed? A missed invalidation shows up here as a
//! pixel difference, not as an intermittent smear on a panel three weeks later.

use denise::{
    BufferAge, Color, ElementState, Frame, InputEvent, Modifiers, PixelFormat, Point,
    PointerButton, Radius, Rect, Role, Size, Surface, SurfaceError, Theme, theme,
};
use denise_render::Pen;
use denise_ui::widget::{Event, EventCtx, Handled, PaintCtx, VisualState, Widget};
use denise_ui::widgets::Panel;
use denise_ui::{NodeId, Ui};

const SIZE: Size = Size::new(320, 200);
/// Padding so nothing may assume rows are contiguous, exactly as on real hardware.
const STRIDE: u32 = SIZE.width + 5;

// ---------------------------------------------------------------- test surface

/// A double-buffered in-memory surface reporting honest buffer ages.
struct Buffers {
    pixels: [Vec<u32>; 2],
    presented_at: [Option<u64>; 2],
    frame: u64,
    current: usize,
}

impl Buffers {
    fn new() -> Self {
        let len = STRIDE as usize * SIZE.height as usize;
        Self {
            pixels: [vec![0x00DE_AD00; len], vec![0x00DE_AD00; len]],
            presented_at: [None; 2],
            frame: 0,
            current: 0,
        }
    }

    /// Visible pixels of the buffer most recently presented, padding removed.
    fn visible(&self) -> Vec<u32> {
        let index = (self.current + 1) % 2;
        self.pixels[index]
            .chunks(STRIDE as usize)
            .take(SIZE.height as usize)
            .flat_map(|row| &row[..SIZE.width as usize])
            .copied()
            .collect()
    }
}

impl Surface for Buffers {
    fn size(&self) -> Size {
        SIZE
    }
    fn scale_factor(&self) -> f32 {
        1.0
    }
    fn format(&self) -> PixelFormat {
        PixelFormat::Xrgb8888
    }
    fn acquire(&mut self) -> Result<Frame<'_>, SurfaceError> {
        let age = match self.presented_at[self.current] {
            Some(then) => BufferAge::Frames((self.frame + 1 - then) as u32),
            None => BufferAge::Undefined,
        };
        Frame::new(
            &mut self.pixels[self.current],
            SIZE,
            STRIDE,
            PixelFormat::Xrgb8888,
            age,
        )
    }
    fn present(&mut self, _damage: &[Rect]) -> Result<(), SurfaceError> {
        self.frame += 1;
        self.presented_at[self.current] = Some(self.frame);
        self.current = (self.current + 1) % 2;
        Ok(())
    }
}

/// Paints the tree into a fresh full-surface buffer, ignoring damage entirely.
fn full_repaint<M: 'static>(ui: &mut Ui<M>) -> Vec<u32> {
    let mut pixels = vec![0x00DE_AD00u32; STRIDE as usize * SIZE.height as usize];
    ui.invalidate_all();
    {
        let mut frame = Frame::new(
            &mut pixels,
            SIZE,
            STRIDE,
            PixelFormat::Xrgb8888,
            BufferAge::Undefined,
        )
        .expect("frame");
        ui.paint(&mut frame);
    }
    ui.presented();
    pixels
        .chunks(STRIDE as usize)
        .take(SIZE.height as usize)
        .flat_map(|row| &row[..SIZE.width as usize])
        .copied()
        .collect()
}

/// Renders incrementally, then asserts the result is pixel-identical to a full
/// repaint of the same tree.
#[track_caller]
fn assert_matches_full_repaint<M: 'static>(ui: &mut Ui<M>, buffers: &mut Buffers, what: &str) {
    ui.render(buffers).expect("render");
    let incremental = buffers.visible();
    let expected = full_repaint(ui);
    let mismatches = incremental
        .iter()
        .zip(&expected)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .count();
    if mismatches != 0 {
        let (i, _) = incremental
            .iter()
            .zip(&expected)
            .enumerate()
            .find(|(_, (a, b))| a != b)
            .expect("counted a mismatch");
        panic!(
            "{what}: {mismatches} pixels differ from a full repaint, \
             first at {},{}: got {:08X}, expected {:08X}",
            i % SIZE.width as usize,
            i / SIZE.width as usize,
            incremental[i],
            expected[i],
        );
    }
}

// --------------------------------------------------------------- test widgets

/// A button-shaped widget that records what it was told, so tests can assert on
/// routing without depending on the shipped widgets.
#[derive(Debug)]
struct Probe {
    clicks: u32,
    focus_gained: u32,
    focus_lost: u32,
    focusable: bool,
    color: Role,
}

impl Probe {
    fn interactive(color: Role) -> Self {
        Self {
            clicks: 0,
            focus_gained: 0,
            focus_lost: 0,
            focusable: true,
            color,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Msg {
    Clicked,
}

impl Widget<Msg> for Probe {
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        let mut color = ctx.theme.color(self.color);
        // Every visual state must change the pixels, otherwise the tests below
        // could not tell a missing invalidation from a no-op.
        if ctx.state.contains(VisualState::HOVERED) {
            color = color.mix(Color::WHITE, 64);
        }
        if ctx.state.contains(VisualState::PRESSED) {
            color = color.mix(Color::BLACK, 96);
        }
        if ctx.state.contains(VisualState::FOCUSED) {
            color = color.mix(ctx.theme.color(Role::Accent), 48);
        }
        canvas.fill_rounded_rect(ctx.bounds, ctx.theme.radius(Radius::Box), color);
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, Msg>) -> Handled {
        match event {
            Event::FocusGained => {
                self.focus_gained += 1;
                Handled::No
            }
            Event::FocusLost => {
                self.focus_lost += 1;
                Handled::No
            }
            Event::Input(InputEvent::PointerButton {
                state: ElementState::Up,
                position,
                ..
            }) => {
                if ctx.bounds.contains(*position) {
                    self.clicks += 1;
                    ctx.emit(Msg::Clicked);
                }
                Handled::Yes
            }
            _ => Handled::No,
        }
    }

    fn accepts_pointer(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        self.focusable
    }
}

fn press_at(x: i32, y: i32) -> [InputEvent; 2] {
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
    ]
}

fn release_at(x: i32, y: i32) -> InputEvent {
    InputEvent::PointerButton {
        button: PointerButton::Left,
        state: ElementState::Up,
        position: Point::new(x, y),
        modifiers: Modifiers::NONE,
    }
}

fn tab(shift: bool) -> InputEvent {
    InputEvent::Key {
        code: denise::KeyCode::Tab,
        state: ElementState::Down,
        repeat: false,
        modifiers: if shift {
            Modifiers::SHIFT
        } else {
            Modifiers::NONE
        },
    }
}

/// A tree with two buttons side by side on a panel.
fn two_buttons(theme: Theme) -> (Ui<Msg>, NodeId, NodeId) {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme);
    let root = ui.root();
    let panel = ui
        .add(root, Panel::default(), Rect::new(20, 20, 280, 160))
        .expect("panel");
    let left = ui
        .add(
            panel,
            Probe::interactive(Role::Primary),
            Rect::new(20, 20, 100, 40),
        )
        .expect("left");
    let right = ui
        .add(
            panel,
            Probe::interactive(Role::Secondary),
            Rect::new(140, 20, 100, 40),
        )
        .expect("right");
    (ui, left, right)
}

// -------------------------------------------------------------------- tests

#[test]
fn paint_order_is_parents_then_siblings_by_z() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    // Added in one order, z-ordered into another. `under` covers the same pixels
    // as `over`, so whichever paints last decides the colour.
    let over = ui
        .add(
            root,
            Panel::filled(Role::Primary),
            Rect::new(10, 10, 60, 60),
        )
        .expect("over");
    let under = ui
        .add(root, Panel::filled(Role::Error), Rect::new(10, 10, 60, 60))
        .expect("under");
    ui.set_z(over, 10);
    ui.set_z(under, -10);

    let pixels = full_repaint(&mut ui);
    let at = pixels[40 * SIZE.width as usize + 40];
    assert_eq!(
        at,
        theme::DARK.color(Role::Primary).to_argb8888(),
        "the higher z must paint last"
    );
}

#[test]
fn hit_testing_finds_the_topmost_interactive_node() {
    let (mut ui, left, right) = two_buttons(theme::DARK);
    // A panel over the button must not steal the hit: panels are not interactive.
    let root = ui.root();
    ui.add(root, Panel::default(), Rect::from_size(SIZE))
        .expect("overlay");

    assert_eq!(ui.hit_test(Point::new(70, 60)), Some(left));
    assert_eq!(ui.hit_test(Point::new(190, 60)), Some(right));
    assert_eq!(ui.hit_test(Point::new(5, 5)), None);
}

#[test]
fn children_are_clipped_to_their_parent() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let parent = ui
        .add(
            root,
            Panel::filled(Role::Base200),
            Rect::new(50, 50, 60, 60),
        )
        .expect("parent");
    // A child that hangs well outside the parent, in every direction.
    let child = ui
        .add(
            parent,
            Probe::interactive(Role::Error),
            Rect::new(-40, -40, 200, 200),
        )
        .expect("child");

    assert_eq!(
        ui.hit_test(Point::new(30, 30)),
        None,
        "the part of the child outside the parent must not be hittable"
    );
    assert_eq!(ui.hit_test(Point::new(80, 80)), Some(child));

    let pixels = full_repaint(&mut ui);
    assert_eq!(
        pixels[30 * SIZE.width as usize + 30],
        theme::DARK.color(Role::Base100).to_argb8888(),
        "the child must not paint outside its parent either"
    );
}

#[test]
fn a_click_reaches_the_widget_and_emits_a_message() {
    let (mut ui, left, _) = two_buttons(theme::DARK);
    ui.handle(&press_at(70, 60));
    ui.handle(&[release_at(70, 60)]);

    assert_eq!(ui.widget::<Probe>(left).expect("probe").clicks, 1);
    assert_eq!(ui.drain_messages().collect::<Vec<_>>(), vec![Msg::Clicked]);
    assert_eq!(ui.focused(), Some(left));
}

#[test]
fn releasing_outside_the_pressed_widget_does_not_click_it() {
    let (mut ui, left, _) = two_buttons(theme::DARK);
    ui.handle(&press_at(70, 60));
    ui.handle(&[
        InputEvent::PointerMoved {
            position: Point::new(5, 5),
        },
        release_at(5, 5),
    ]);

    assert_eq!(
        ui.widget::<Probe>(left).expect("probe").clicks,
        0,
        "dragging off a button before releasing must cancel it"
    );
    assert!(ui.messages().is_empty());
}

#[test]
fn tab_walks_focusable_nodes_in_paint_order() {
    let (mut ui, left, right) = two_buttons(theme::DARK);
    ui.handle(&[tab(false)]);
    assert_eq!(ui.focused(), Some(left));
    ui.handle(&[tab(false)]);
    assert_eq!(ui.focused(), Some(right));
    ui.handle(&[tab(false)]);
    assert_eq!(ui.focused(), Some(left), "focus wraps");
    ui.handle(&[tab(true)]);
    assert_eq!(ui.focused(), Some(right), "shift walks backwards");

    assert_eq!(ui.widget::<Probe>(left).expect("probe").focus_gained, 2);
    assert_eq!(ui.widget::<Probe>(left).expect("probe").focus_lost, 2);
}

#[test]
fn a_modal_scene_takes_every_input_from_the_scene_below() {
    let (mut ui, left, _) = two_buttons(theme::DARK);
    ui.handle(&[tab(false)]);
    assert_eq!(ui.focused(), Some(left));

    let dialog_root = ui.push_scene(128);
    let ok = ui
        .add(
            dialog_root,
            Probe::interactive(Role::Success),
            Rect::new(100, 70, 120, 44),
        )
        .expect("ok");

    assert_eq!(ui.focused(), None, "pushing a scene drops focus behind it");
    assert_eq!(
        ui.hit_test(Point::new(70, 60)),
        None,
        "a button in the scene below must not be hittable"
    );
    assert_eq!(ui.hit_test(Point::new(150, 90)), Some(ok));

    ui.handle(&[tab(false)]);
    assert_eq!(
        ui.focused(),
        Some(ok),
        "Tab must not reach behind the modal"
    );

    ui.handle(&press_at(150, 90));
    ui.handle(&[release_at(150, 90)]);
    assert_eq!(ui.widget::<Probe>(ok).expect("probe").clicks, 1);
    assert_eq!(ui.widget::<Probe>(left).expect("probe").clicks, 0);

    assert!(ui.pop_scene());
    assert_eq!(ui.hit_test(Point::new(70, 60)), Some(left));
    assert!(!ui.contains(ok), "popping a scene destroys its nodes");
}

#[test]
fn a_stale_id_resolves_to_nothing_rather_than_the_wrong_widget() {
    let (mut ui, left, _) = two_buttons(theme::DARK);
    assert!(ui.remove(left));
    assert!(!ui.contains(left));
    assert!(ui.widget::<Probe>(left).is_none());
    assert_eq!(ui.hit_test(Point::new(70, 60)), None);
}

#[test]
fn removing_the_focused_node_clears_focus() {
    let (mut ui, left, _) = two_buttons(theme::DARK);
    ui.handle(&press_at(70, 60));
    assert_eq!(ui.focused(), Some(left));
    ui.remove(left);
    assert_eq!(ui.focused(), None);
    assert_eq!(ui.hovered(), None);
}

#[test]
fn disabling_a_parent_disables_its_children() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let panel = ui
        .add(root, Panel::default(), Rect::new(20, 20, 280, 160))
        .expect("panel");
    let button = ui
        .add(
            panel,
            Probe::interactive(Role::Primary),
            Rect::new(20, 20, 100, 40),
        )
        .expect("button");

    ui.set_enabled(panel, false);
    assert_eq!(ui.hit_test(Point::new(70, 60)), None);
    ui.handle(&[tab(false)]);
    assert_eq!(ui.focused(), None);

    ui.set_enabled(panel, true);
    assert_eq!(ui.hit_test(Point::new(70, 60)), Some(button));
}

#[test]
fn what_was_asked_of_a_node_can_be_asked_back() {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let panel = ui
        .add(root, Panel::default(), Rect::new(20, 20, 280, 160))
        .expect("panel");
    let button = ui
        .add(
            panel,
            Probe::interactive(Role::Primary),
            Rect::new(20, 20, 100, 40),
        )
        .expect("button");

    assert!(ui.enabled(button), "a node starts live");
    assert_eq!(ui.tooltip(button), None, "a node starts without one");

    ui.set_tooltip(button, "Write the form to disk");
    assert_eq!(ui.tooltip(button), Some("Write the form to disk"));

    ui.set_enabled(button, false);
    assert!(!ui.enabled(button));

    // Disabling the *parent* greys the child without changing what was asked of
    // the child, which is what a caller putting it back needs to know.
    ui.set_enabled(button, true);
    ui.set_enabled(panel, false);
    assert!(!ui.enabled(panel));
    assert!(ui.enabled(button), "the child was never disabled itself");

    ui.clear_tooltip(button);
    assert_eq!(ui.tooltip(button), None);
}

// ------------------------------------------- the damage-equals-truth tests

#[test]
fn moving_a_node_repaints_what_it_left_behind() {
    let (mut ui, left, _) = two_buttons(theme::DARK);
    let mut buffers = Buffers::new();
    assert_matches_full_repaint(&mut ui, &mut buffers, "first frame");

    for step in 1..=6 {
        ui.set_layout(left, Rect::new(20 + step * 13, 20, 100, 40));
        assert_matches_full_repaint(&mut ui, &mut buffers, "after moving a node");
    }
}

#[test]
fn hover_and_press_repaint_without_the_application_asking() {
    let (mut ui, _, _) = two_buttons(theme::DARK);
    let mut buffers = Buffers::new();
    assert_matches_full_repaint(&mut ui, &mut buffers, "first frame");

    // The exact sequence that produced a stale-colour bug on hardware: press,
    // release without moving, and expect the release's colour change to show.
    ui.handle(&[InputEvent::PointerMoved {
        position: Point::new(70, 60),
    }]);
    assert_matches_full_repaint(&mut ui, &mut buffers, "after hovering");

    ui.handle(&press_at(70, 60));
    assert_matches_full_repaint(&mut ui, &mut buffers, "after pressing");

    ui.handle(&[release_at(70, 60)]);
    assert_matches_full_repaint(&mut ui, &mut buffers, "after releasing in place");
}

#[test]
fn the_cursor_sprite_repairs_the_pixels_it_leaves() {
    let (mut ui, _, _) = two_buttons(theme::DARK);
    let mut buffers = Buffers::new();
    assert_matches_full_repaint(&mut ui, &mut buffers, "first frame");

    for x in (30..280).step_by(17) {
        ui.handle(&[InputEvent::PointerMoved {
            position: Point::new(x, 100),
        }]);
        assert_matches_full_repaint(&mut ui, &mut buffers, "after moving the pointer");
    }
}

/// Left alone the tree decides: a pointer reveals the sprite, a finger hides it.
/// That is right for a panel with nothing underneath it to draw a cursor.
#[test]
fn the_tree_shows_the_sprite_for_a_pointer_and_not_for_a_finger() {
    let (mut ui, _, _) = two_buttons(theme::DARK);
    assert!(!ui.cursor().visible, "nothing has moved yet");

    ui.handle(&[InputEvent::PointerMoved {
        position: Point::new(70, 60),
    }]);
    assert!(ui.cursor().visible, "a pointer moved and drew nothing");

    ui.handle(&[InputEvent::TouchDown {
        id: 1,
        position: Point::new(70, 60),
    }]);
    assert!(!ui.cursor().visible, "a finger left a pointer behind");
}

/// An embedded host has a system cursor already, and a second one compositing a
/// frame behind it is worse than none. So `show_cursor` is a decision, not a
/// suggestion: once made, no amount of pointer motion overrides it.
#[test]
fn an_explicit_show_cursor_survives_every_later_event() {
    let (mut ui, _, _) = two_buttons(theme::DARK);
    ui.show_cursor(false);

    for x in (30..280).step_by(37) {
        ui.handle(&[InputEvent::PointerMoved {
            position: Point::new(x, 100),
        }]);
        assert!(
            !ui.cursor().visible,
            "the tree drew a cursor over the host's"
        );
    }

    // And the other direction: a kiosk that wants the sprite always, including
    // under a finger, gets to say so.
    ui.show_cursor(true);
    ui.handle(&[InputEvent::TouchDown {
        id: 1,
        position: Point::new(70, 60),
    }]);
    assert!(ui.cursor().visible, "a finger overrode an explicit request");
}

#[test]
fn structural_changes_repaint_correctly() {
    let (mut ui, left, right) = two_buttons(theme::DARK);
    let mut buffers = Buffers::new();
    assert_matches_full_repaint(&mut ui, &mut buffers, "first frame");

    ui.set_visible(left, false);
    assert_matches_full_repaint(&mut ui, &mut buffers, "after hiding a node");

    ui.set_visible(left, true);
    assert_matches_full_repaint(&mut ui, &mut buffers, "after showing it again");

    ui.set_z(left, 5);
    ui.set_layout(right, Rect::new(60, 20, 100, 40));
    assert_matches_full_repaint(&mut ui, &mut buffers, "after overlapping siblings");

    ui.remove(right);
    assert_matches_full_repaint(&mut ui, &mut buffers, "after removing a node");

    let root = ui.root();
    ui.add(
        root,
        Panel::filled(Role::Accent),
        Rect::new(200, 120, 80, 50),
    );
    assert_matches_full_repaint(&mut ui, &mut buffers, "after adding a node");
}

#[test]
fn a_modal_backdrop_repaints_correctly_over_a_dirty_scene() {
    let (mut ui, left, _) = two_buttons(theme::DARK);
    let mut buffers = Buffers::new();
    assert_matches_full_repaint(&mut ui, &mut buffers, "first frame");

    let dialog = ui.push_scene(160);
    ui.add(dialog, Panel::default(), Rect::new(80, 60, 160, 80));
    assert_matches_full_repaint(&mut ui, &mut buffers, "after opening a modal");

    // Something small changing inside the dialog must not disturb the backdrop.
    ui.handle(&[InputEvent::PointerMoved {
        position: Point::new(150, 100),
    }]);
    assert_matches_full_repaint(&mut ui, &mut buffers, "with a modal open");

    assert!(ui.pop_scene());
    assert_matches_full_repaint(&mut ui, &mut buffers, "after closing the modal");

    ui.set_layout(left, Rect::new(30, 30, 100, 40));
    assert_matches_full_repaint(&mut ui, &mut buffers, "after the modal is gone");
}

#[test]
fn changing_the_theme_repaints_everything() {
    let (mut ui, _, _) = two_buttons(theme::DARK);
    let mut buffers = Buffers::new();
    assert_matches_full_repaint(&mut ui, &mut buffers, "first frame");
    ui.set_theme(theme::LIGHT);
    assert_matches_full_repaint(&mut ui, &mut buffers, "after a theme swap");
}

#[test]
fn an_idle_tree_draws_no_frame_at_all() {
    let (mut ui, _, _) = two_buttons(theme::DARK);
    let mut buffers = Buffers::new();
    assert!(
        ui.render(&mut buffers).expect("render"),
        "first frame draws"
    );
    assert!(
        !ui.render(&mut buffers).expect("render"),
        "nothing changed, so nothing should be drawn"
    );

    // A pointer move that lands on the same pixel is not a change either.
    ui.handle(&[InputEvent::PointerMoved {
        position: Point::new(70, 60),
    }]);
    assert!(ui.render(&mut buffers).expect("render"));
    ui.handle(&[InputEvent::PointerMoved {
        position: Point::new(70, 60),
    }]);
    assert!(!ui.render(&mut buffers).expect("render"));
}

#[test]
fn damage_stays_proportional_to_what_moved() {
    let (mut ui, _, _) = two_buttons(theme::DARK);
    let mut buffers = Buffers::new();
    // Both buffers have to have been presented once before either reports a usable
    // age; until then every frame is honestly a full repaint.
    for x in 100..106 {
        ui.handle(&[InputEvent::PointerMoved {
            position: Point::new(x, 100),
        }]);
        ui.render(&mut buffers).expect("render");
    }

    let area: u64 = ui.damage().iter().map(Rect::area).sum();
    let surface = Rect::from_size(SIZE).area();
    assert!(
        area * 20 < surface,
        "moving the pointer repainted {area} of {surface} pixels; \
         a cursor move should cost two sprite-sized rectangles"
    );
}

#[test]
fn the_shipped_widgets_damage_exactly_what_they_change() {
    use denise::KeyCode;
    use denise_ui::widgets::{Button, Label, TextInput};

    let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let card = ui
        .add(root, Panel::default(), Rect::new(10, 10, 300, 180))
        .expect("card");
    let label = ui
        .add(card, Label::new("Name"), Rect::new(16, 10, 120, 24))
        .expect("label");
    let field = ui
        .add(
            card,
            TextInput::<Msg>::new().with_placeholder("type here"),
            Rect::new(16, 40, 260, 36),
        )
        .expect("field");
    ui.add(
        card,
        Button::new("Save", Msg::Clicked),
        Rect::new(16, 120, 120, 40),
    )
    .expect("save");

    let mut buffers = Buffers::new();
    assert_matches_full_repaint(&mut ui, &mut buffers, "the form as built");

    // Tab to the field, type into it, and check every intermediate frame.
    ui.handle(&[tab(false)]);
    assert_matches_full_repaint(&mut ui, &mut buffers, "after focusing the field");

    for ch in "Kjærlighet på Øy, and then some more than fits".chars() {
        ui.handle(&[InputEvent::Text { ch }]);
        assert_matches_full_repaint(&mut ui, &mut buffers, "after a keystroke");
    }

    for _ in 0..8 {
        ui.handle(&[InputEvent::Key {
            code: KeyCode::ArrowLeft,
            state: ElementState::Down,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        assert_matches_full_repaint(&mut ui, &mut buffers, "after moving the caret");
    }

    // The caret blinking is the one thing that repaints on a timer.
    for step in 1..=4 {
        ui.tick(step * 500);
        assert_matches_full_repaint(&mut ui, &mut buffers, "after a blink");
    }

    ui.handle(&[tab(false)]);
    assert_matches_full_repaint(&mut ui, &mut buffers, "after tabbing to the button");

    ui.handle(&press_at(80, 180));
    assert_matches_full_repaint(&mut ui, &mut buffers, "with the button held");
    ui.handle(&[release_at(80, 180)]);
    assert_matches_full_repaint(&mut ui, &mut buffers, "after releasing the button");

    ui.widget_mut::<Label>(label)
        .expect("label")
        .set_text("Navn");
    assert_matches_full_repaint(&mut ui, &mut buffers, "after changing a label");

    assert!(
        ui.widget::<TextInput<Msg>>(field)
            .expect("field")
            .text()
            .starts_with("Kjærlighet"),
        "the field should have kept what was typed"
    );
}
