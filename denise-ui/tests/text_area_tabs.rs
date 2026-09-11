//! A tab reaches its stop everywhere the text area measures a line: where
//! the text is drawn, and where the caret, the selection, the marks and a
//! click are.

use std::borrow::Cow;
use std::ops::Range;

use denise::{
    BufferAge, ElementState, Frame, InputEvent, Modifiers, PixelFormat, Point, PointerButton, Rect,
    Size, theme,
};
use denise_ui::widgets::{Pos, TextArea, TextBuffer, TextDocument};
use denise_ui::{NodeId, Ui};

const SIZE: Size = Size::new(400, 120);
const AREA: Rect = Rect::new(10, 10, 380, 100);

/// A buffer that marks every `x`, so the marks are checked against tabs too.
struct MarkX(TextBuffer);

impl TextDocument for MarkX {
    fn line_count(&mut self) -> Option<usize> {
        self.0.line_count()
    }

    fn known_lines(&mut self) -> usize {
        self.0.known_lines()
    }

    fn line(&mut self, n: usize) -> Option<Cow<'_, str>> {
        self.0.line(n)
    }

    fn insert(&mut self, at: Pos, text: &str) {
        self.0.insert(at, text);
    }

    fn delete(&mut self, from: Pos, to: Pos) {
        self.0.delete(from, to);
    }

    fn highlights(&mut self, _n: usize, line: &str, out: &mut Vec<Range<usize>>) {
        out.extend(line.match_indices('x').map(|(i, _)| i..i + 1));
    }
}

type Editor = TextArea<(), MarkX>;

fn editor(text: &str, tab_width: u8) -> (Ui<()>, NodeId) {
    let mut ui: Ui<()> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let area = Editor::new(MarkX(TextBuffer::from_text(text))).with_tab_width(tab_width);
    let id = ui.add(root, area, AREA).expect("editor");
    (ui, id)
}

fn render(ui: &mut Ui<()>) -> Vec<u32> {
    let mut pixels = vec![0u32; (SIZE.width * SIZE.height) as usize];
    let mut frame = Frame::new(
        &mut pixels,
        SIZE,
        SIZE.width,
        PixelFormat::Xrgb8888,
        BufferAge::Undefined,
    )
    .expect("frame");
    ui.paint(&mut frame);
    pixels
}

/// Asserts that `tabbed` and `spaced` draw the same pixels, with the caret at
/// the end of the first line and that line's last character selected.
#[track_caller]
fn looks_the_same(tabbed: &str, spaced: &str, tab_width: u8) {
    let frame = |text: &str| {
        let (mut ui, id) = editor(text, tab_width);
        ui.focus(Some(id));
        let first = text.lines().next().unwrap_or("");
        let last = first.char_indices().last().map_or(0, |(i, _)| i);
        ui.widget_mut::<Editor>(id)
            .expect("editor")
            .select_range(Pos::new(0, last), Pos::new(0, first.len()));
        render(&mut ui)
    };
    assert!(
        frame(tabbed) == frame(spaced),
        "{tabbed:?} is not drawn like {spaced:?} with stops every {tab_width}"
    );
}

#[test]
fn a_tab_reaches_the_next_stop() {
    looks_the_same("\tx", "    x", 4);
    looks_the_same("ab\tx", "ab  x", 4);
    looks_the_same("abcd\tx", "abcd    x", 4);
    looks_the_same("a\t\tx", "a       x", 4);
    looks_the_same("\tx", "        x", 8);
    looks_the_same("\tx", " x", 1);
    looks_the_same("x\ty\n\tz x", "x   y\n    z x", 4);
}

fn click(at: Point) -> [InputEvent; 2] {
    [
        InputEvent::PointerButton {
            button: PointerButton::Left,
            state: ElementState::Down,
            position: at,
            modifiers: Modifiers::NONE,
        },
        InputEvent::PointerButton {
            button: PointerButton::Left,
            state: ElementState::Up,
            position: at,
            modifiers: Modifiers::NONE,
        },
    ]
}

#[test]
fn a_click_lands_on_the_character_a_tab_pushed_along() {
    let column_at = |text: &str, x: i32| {
        let (mut ui, id) = editor(text, 4);
        render(&mut ui);
        ui.handle(&click(Point::new(x, AREA.y + 5)));
        ui.widget::<Editor>(id).expect("editor").caret().col
    };
    // `\tx` has boundaries at 0, the stop and one past it; `    x` has one
    // every space. Wherever the spaced line puts the caret before or after
    // its `x`, the tabbed one must put it before or after its own.
    let mut reached = [false; 3];
    for x in AREA.x..AREA.right() - 20 {
        let tabbed = column_at("\tx", x);
        match column_at("    x", x) {
            0 | 1 => assert_eq!(tabbed, 0, "at x {x}"),
            4 => assert_eq!(tabbed, 1, "at x {x}"),
            5 => assert_eq!(tabbed, 2, "at x {x}"),
            _ => continue,
        }
        reached[tabbed] = true;
    }
    assert_eq!(
        reached, [true; 3],
        "every column of the tabbed line was clicked"
    );
}
