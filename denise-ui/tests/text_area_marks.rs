//! What a document asks the text area to mark is drawn behind its text.

use std::borrow::Cow;
use std::ops::Range;

use denise::{BufferAge, Frame, PixelFormat, Rect, Size, theme};
use denise_ui::widgets::{Pos, TextArea, TextBuffer, TextDocument};
use denise_ui::{TextEngine, TextStyle, Ui};

const SIZE: Size = Size::new(400, 300);
const AREA: Rect = Rect::new(10, 10, 380, 280);

/// A buffer that marks every occurrence of one word, the way a find does.
struct Marking {
    buffer: TextBuffer,
    word: &'static str,
}

impl TextDocument for Marking {
    fn line_count(&mut self) -> Option<usize> {
        self.buffer.line_count()
    }

    fn known_lines(&mut self) -> usize {
        self.buffer.known_lines()
    }

    fn line(&mut self, n: usize) -> Option<Cow<'_, str>> {
        self.buffer.line(n)
    }

    fn insert(&mut self, at: Pos, text: &str) {
        self.buffer.insert(at, text);
    }

    fn delete(&mut self, from: Pos, to: Pos) {
        self.buffer.delete(from, to);
    }

    fn highlights(&mut self, _n: usize, line: &str, out: &mut Vec<Range<usize>>) {
        if !self.word.is_empty() {
            out.extend(line.match_indices(self.word).map(|(i, w)| i..i + w.len()));
        }
    }
}

type Editor = TextArea<(), Marking>;

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

/// The pixels of the `index`-th text row, the built-in font's height tall.
fn row(pixels: &[u32], index: i32) -> &[u32] {
    let height = TextEngine::new().line_height(TextStyle::built_in(16));
    let top = (AREA.y + index * height) as usize;
    let width = SIZE.width as usize;
    &pixels[top * width..(top + height as usize) * width]
}

#[test]
fn every_mark_is_drawn_behind_the_text_on_its_own_line() {
    let doc = Marking {
        buffer: TextBuffer::from_text("alpha beta\ngamma\nbeta and beta"),
        word: "",
    };
    let mut ui: Ui<()> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let id = ui.add(root, Editor::new(doc), AREA).expect("editor");
    let plain = render(&mut ui);

    ui.widget_mut::<Editor>(id)
        .expect("editor")
        .document_mut()
        .word = "beta";
    let marked = render(&mut ui);

    let changed = |index| {
        row(&plain, index)
            .iter()
            .zip(row(&marked, index))
            .filter(|(a, b)| a != b)
            .count()
    };
    assert!(
        changed(0) > 0,
        "the first line has a beta, and it is marked"
    );
    assert_eq!(changed(1), 0, "the second line has none");
    assert!(
        changed(2) > changed(0),
        "two marks on the third line cover more than the one on the first"
    );

    // And the marks go when the document stops asking for them.
    ui.widget_mut::<Editor>(id)
        .expect("editor")
        .document_mut()
        .word = "";
    assert_eq!(render(&mut ui), plain);
}
