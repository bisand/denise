//! A menu behaves the way a desktop's do: a submenu opens under a resting
//! pointer and closes when the pointer settles on another row, a release
//! chooses, a press outside dismisses, and sliding along the bar switches.

use denise::{ElementState, InputEvent, KeyCode, Modifiers, Point, Rect, Size, theme};
use denise_ui::widgets::{Menu, MenuBar, MenuEvent, MenuItem, open_menu, title_layout};
use denise_ui::{NodeId, Ui};

const SIZE: Size = Size::new(640, 400);
const BAR: Rect = Rect::new(0, 0, 640, 28);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Msg {
    Open(usize),
    Menu(MenuEvent),
}

/// File: Open…, ─, Recent ▸ (a, b, ─, Clear), ─, Quit. Numbered depth first:
/// Open 0, rule 1, Recent 2, a 3, b 4, rule 5, Clear 6, rule 7, Quit 8.
fn file_menu() -> Vec<MenuItem> {
    vec![
        MenuItem::new("Open…").with_shortcut("Cmd+O"),
        MenuItem::separator(),
        MenuItem::submenu(
            "Recent",
            [
                MenuItem::new("a.log"),
                MenuItem::new("b.log"),
                MenuItem::separator(),
                MenuItem::new("Clear Recent"),
            ],
        ),
        MenuItem::separator(),
        MenuItem::new("Quit").with_shortcut("Cmd+Q"),
    ]
}

struct Fixture {
    ui: Ui<Msg>,
    bar: NodeId,
    menu: NodeId,
}

impl Fixture {
    fn open() -> Self {
        let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
        let root = ui.root();
        let bar = ui
            .add(root, MenuBar::new(["File", "Edit", "View"], Msg::Open), BAR)
            .expect("bar");
        ui.show_cursor(false);
        ui.tick(0);
        let menu = open_menu(&mut ui, bar, 0, &file_menu(), Msg::Menu).expect("menu");
        ui.drain_messages();
        Self { ui, bar, menu }
    }

    fn menu(&self) -> &Menu<Msg> {
        self.ui.widget::<Menu<Msg>>(self.menu).expect("menu")
    }

    fn panels(&self) -> Vec<Rect> {
        self.menu().panels().collect()
    }

    fn centre(&self, panel: usize, row: usize) -> Point {
        let r = self.menu().row_rect(panel, row).expect("row");
        Point::new(r.x + r.width / 2, r.y + r.height / 2)
    }

    fn move_to(&mut self, p: Point) {
        self.ui.handle(&[InputEvent::PointerMoved { position: p }]);
    }

    fn click(&mut self, p: Point) {
        self.move_to(p);
        self.ui.handle(&[
            InputEvent::PointerButton {
                button: denise::PointerButton::Left,
                state: ElementState::Down,
                position: p,
                modifiers: Modifiers::default(),
            },
            InputEvent::PointerButton {
                button: denise::PointerButton::Left,
                state: ElementState::Up,
                position: p,
                modifiers: Modifiers::default(),
            },
        ]);
    }

    fn key(&mut self, code: KeyCode) {
        self.ui.handle(&[InputEvent::Key {
            code,
            state: ElementState::Down,
            repeat: false,
            modifiers: Modifiers::default(),
        }]);
    }

    fn messages(&mut self) -> Vec<Msg> {
        self.ui.drain_messages().collect()
    }

    fn title(&mut self, index: usize) -> Rect {
        let titles: Vec<String> = self
            .ui
            .widget::<MenuBar<Msg>>(self.bar)
            .expect("bar")
            .titles()
            .to_vec();
        let style = self
            .ui
            .widget::<MenuBar<Msg>>(self.bar)
            .expect("bar")
            .style();
        title_layout(&titles, BAR, style, self.ui.text_mut())[index]
    }
}

#[test]
fn the_root_hangs_under_its_title_and_a_release_on_a_row_chooses_it() {
    let mut f = Fixture::open();
    let panels = f.panels();
    assert_eq!(panels.len(), 1);
    let title = f.title(0);
    assert_eq!(panels[0].x, title.x, "aligned with the title");
    assert_eq!(panels[0].y, BAR.bottom(), "directly under the bar");

    let quit = f.centre(0, 4);
    f.click(quit);
    assert_eq!(f.messages(), vec![Msg::Menu(MenuEvent::Picked(8))]);
}

#[test]
fn a_submenu_opens_beside_the_row_the_pointer_rests_on_and_its_rows_count_after_it() {
    let mut f = Fixture::open();
    f.move_to(f.centre(0, 2));
    let panels = f.panels();
    assert_eq!(panels.len(), 2, "the submenu opened");
    assert!(panels[1].x > panels[0].x, "beside the root, to the right");
    let recent = f.menu().row_rect(0, 2).expect("recent");
    let first = f.menu().row_rect(1, 0).expect("first of the submenu");
    assert_eq!(
        first.y, recent.y,
        "its first row level with the row that opened it"
    );

    // Into the submenu and onto its second row: b.log is row 4 overall.
    let b = f.centre(1, 1);
    f.click(b);
    assert_eq!(f.messages(), vec![Msg::Menu(MenuEvent::Picked(4))]);
    // And Clear Recent, past the rule, is 6.
    let clear = f.centre(1, 3);
    f.click(clear);
    assert_eq!(f.messages(), vec![Msg::Menu(MenuEvent::Picked(6))]);
}

#[test]
fn a_submenu_closes_once_the_pointer_has_rested_on_another_row() {
    let mut f = Fixture::open();
    f.move_to(f.centre(0, 2));
    assert_eq!(f.panels().len(), 2);

    // Cutting the corner over Quit on the way to the submenu: still open.
    f.move_to(f.centre(0, 4));
    f.ui.tick(60);
    assert_eq!(
        f.panels().len(),
        2,
        "not yet: the pointer may be passing through"
    );
    // Into the submenu in time: it stays, and the pending close is forgotten.
    f.move_to(f.centre(1, 0));
    f.ui.tick(400);
    assert_eq!(f.panels().len(), 2, "the pointer made it into the submenu");

    // Back out and resting on Quit: closes.
    f.move_to(f.centre(0, 4));
    f.ui.tick(500);
    assert_eq!(f.panels().len(), 2);
    f.ui.tick(700);
    assert_eq!(
        f.panels().len(),
        1,
        "closed after the pointer rested elsewhere"
    );
}

#[test]
fn a_release_over_a_separator_or_a_submenu_row_chooses_nothing() {
    let mut f = Fixture::open();
    f.click(f.centre(0, 1));
    assert_eq!(f.messages(), vec![]);
    f.click(f.centre(0, 2));
    assert_eq!(
        f.messages(),
        vec![],
        "a submenu row opens rather than picks"
    );
    assert_eq!(f.panels().len(), 2);
}

#[test]
fn a_press_outside_dismisses_and_a_press_on_the_open_title_does_too() {
    let mut f = Fixture::open();
    f.click(Point::new(600, 380));
    assert_eq!(f.messages(), vec![Msg::Menu(MenuEvent::Dismissed)]);

    let mut f = Fixture::open();
    let title = f.title(0);
    f.click(Point::new(title.x + 2, title.y + 2));
    assert_eq!(f.messages(), vec![Msg::Menu(MenuEvent::Dismissed)]);
}

#[test]
fn sliding_along_the_bar_reports_the_title_once() {
    let mut f = Fixture::open();
    let edit = f.title(1);
    f.move_to(Point::new(edit.x + 3, edit.y + 3));
    f.move_to(Point::new(edit.x + 6, edit.y + 4));
    assert_eq!(f.messages(), vec![Msg::Menu(MenuEvent::Title(1))]);
    // Back over the open title says nothing; on to View says so.
    let file = f.title(0);
    f.move_to(Point::new(file.x + 3, file.y + 3));
    let view = f.title(2);
    f.move_to(Point::new(view.x + 3, view.y + 3));
    assert_eq!(f.messages(), vec![Msg::Menu(MenuEvent::Title(2))]);
}

#[test]
fn the_keyboard_walks_past_rules_opens_submenus_and_chooses() {
    let mut f = Fixture::open();
    f.key(KeyCode::ArrowDown); // Open…
    f.key(KeyCode::ArrowDown); // over the rule, onto Recent
    f.key(KeyCode::ArrowRight);
    assert_eq!(f.panels().len(), 2, "Right opened the submenu");
    f.key(KeyCode::ArrowDown); // b.log
    f.key(KeyCode::Enter);
    assert_eq!(f.messages(), vec![Msg::Menu(MenuEvent::Picked(4))]);

    f.key(KeyCode::ArrowLeft);
    assert_eq!(f.panels().len(), 1, "Left closed it");
    f.key(KeyCode::ArrowLeft);
    assert_eq!(
        f.messages(),
        vec![Msg::Menu(MenuEvent::Title(2))],
        "Left at the root goes to the previous title, wrapping"
    );
    // The root kept its selection on Recent while the submenu was up.
    f.key(KeyCode::ArrowDown); // over the rule, onto Quit
    f.key(KeyCode::Enter);
    assert_eq!(f.messages(), vec![Msg::Menu(MenuEvent::Picked(8))]);
}
