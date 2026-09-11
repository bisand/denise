//! A multi-line text editor that edits through a document it does not own.
//!
//! # Why the widget does not hold the text
//!
//! A [`TextInput`](super::TextInput) owns a `String`, and for a name or a
//! setpoint that is right. An editor cannot: the file it is showing may be
//! gigabytes, and a widget that held it would have decided, on behalf of every
//! application, that a file is loaded before it is shown. So `TextArea` asks a
//! [`TextDocument`] for the lines it is about to draw and nothing else, and
//! tells it where to insert and what to delete. The one that ships,
//! [`TextBuffer`], is a `Vec<String>` and is what a form gets; an application
//! with a file too big for that implements the trait over whatever it has.
//!
//! The document's line count is an `Option`, which is the one place a big
//! file's shape leaks into the trait: a document still counting its lines
//! answers `None`, the widget numbers and scrolls the lines it has been told
//! are known, and the total arrives when it arrives.
//!
//! # What it does not do
//!
//! No wrapping: a line is as wide as it is, and the view scrolls sideways to
//! follow the caret. No clipboard of its own — this crate has no system to ask
//! — so copy, cut and paste are *messages*: the widget hands the application
//! the text it copied, or asks for the text to paste, and the application
//! answers through [`TextArea::insert_text`]. No Tab: the tree owns Tab for
//! focus stepping, and an editor that took it would trap the keyboard.

use alloc::borrow::Cow;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::{Cell, RefCell, RefMut};
use core::ops::Range;

use denise::Pen;
use denise::{
    Color, ElementState, InputEvent, KeyCode, Modifiers, Point, PointerButton, Rect, Role,
};
use denise_text::{TextEngine, TextStyle};

use crate::motion::Wake;
use crate::widget::{
    Animation, Event, EventCtx, Handled, MeasureCtx, Measured, Offer, PaintCtx, VisualState, Widget,
};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Property, PropertyKind, Value,
};
use crate::widgets::style::{DOUBLE_CLICK_MS, muted};

/// Half-period of the caret blink, in milliseconds.
const BLINK_MS: u64 = 500;

/// A place in a document: a line, and a byte offset into it.
///
/// Bytes rather than characters, because that is what a `&str` is sliced by,
/// and a column that counted characters would cost a walk of the line to use.
/// Every position the widget makes lands on a character boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    /// 0-based line.
    pub line: usize,
    /// Byte offset into the line.
    pub col: usize,
}

impl Pos {
    /// The start of the document.
    pub const ZERO: Self = Self { line: 0, col: 0 };

    /// A position.
    #[must_use]
    pub const fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

/// A coloured run of one line, for syntax highlighting.
///
/// A colour rather than a theme role, because a highlighting scheme has its
/// own palette and the document is the one that knows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// Byte offset the run starts at.
    pub start: usize,
    /// Byte offset it ends at, exclusive.
    pub end: usize,
    /// What to draw it in.
    pub color: Color,
}

/// What a [`TextArea`] edits.
///
/// Every method takes `&mut self`, because a document over a file may have
/// to read to answer and may cache what it read. The widget keeps the document
/// in a [`RefCell`] so painting, which is `&self`, can ask too.
pub trait TextDocument: 'static {
    /// How many lines there are, or `None` while that is still being counted.
    ///
    /// A document always has at least one line: a trailing newline ends a
    /// line and starts an empty last one.
    fn line_count(&mut self) -> Option<usize>;

    /// Lines the document can hand over now. Equal to [`line_count`] once it
    /// is known, and growing until then.
    ///
    /// [`line_count`]: TextDocument::line_count
    fn known_lines(&mut self) -> usize;

    /// Line `n` without its newline, or `None` if it is not (yet) known.
    fn line(&mut self, n: usize) -> Option<Cow<'_, str>>;

    /// Inserts `text`, which may hold newlines, at `at`.
    fn insert(&mut self, at: Pos, text: &str);

    /// Deletes `[from, to)`. `from <= to`; a range across lines joins them.
    fn delete(&mut self, from: Pos, to: Pos);

    /// Takes back the last edit. `false` if there was none.
    fn undo(&mut self) -> bool {
        false
    }

    /// Redoes the last edit undone. `false` if there was none.
    fn redo(&mut self) -> bool {
        false
    }

    /// The coloured runs of line `n`, appended to `out` in ascending order
    /// without overlaps. Text between runs is drawn in the theme's content
    /// colour. The default highlights nothing.
    fn spans(&mut self, n: usize, out: &mut Vec<Span>) {
        let _ = (n, out);
    }

    /// Byte ranges of line `n` to mark behind the text — every match of a
    /// search, say — appended to `out` in ascending order without overlaps.
    /// `line` is the line's text as the widget already has it, so marking
    /// costs no second read. A range out of order, past the end or not on a
    /// character boundary is skipped; the selection is drawn over the marks.
    /// The default marks nothing.
    fn highlights(&mut self, n: usize, line: &str, out: &mut Vec<Range<usize>>) {
        let _ = (n, line, out);
    }
}

/// Where `text` inserted at `at` ends.
#[must_use]
pub fn end_of(at: Pos, text: &str) -> Pos {
    match text.rfind('\n') {
        None => Pos::new(at.line, at.col + text.len()),
        Some(last) => {
            let newlines = text.bytes().filter(|&b| b == b'\n').count();
            Pos::new(at.line + newlines, text.len() - last - 1)
        }
    }
}

/// One edit, kept so it can be taken back.
#[derive(Clone, Debug)]
struct Edit {
    at: Pos,
    text: String,
    inserted: bool,
}

/// A [`TextDocument`] held in memory as a line per `String`.
///
/// What a form gets, and what an application with an ordinary amount of text
/// wants. Undo is a journal of the edits, so it costs what was typed rather
/// than a copy of the text per keystroke; consecutive typing on one line is
/// one entry.
#[derive(Clone, Debug)]
pub struct TextBuffer {
    lines: Vec<String>,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
}

impl TextBuffer {
    /// An empty buffer: one empty line.
    #[must_use]
    pub fn new() -> Self {
        Self::from_text("")
    }

    /// A buffer holding `text`.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        Self {
            lines: text.split('\n').map(String::from).collect(),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// The whole text, lines joined by `\n`.
    #[must_use]
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// The lines.
    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Inserts without recording, and returns where the text ends.
    fn splice_in(&mut self, at: Pos, text: &str) -> Pos {
        let line = &mut self.lines[at.line];
        let tail = line.split_off(at.col);
        let mut parts = text.split('\n');
        line.push_str(parts.next().unwrap_or(""));
        let mut cursor = at.line;
        for part in parts {
            cursor += 1;
            self.lines.insert(cursor, String::from(part));
        }
        let end = Pos::new(cursor, self.lines[cursor].len());
        self.lines[cursor].push_str(&tail);
        end
    }

    /// Deletes without recording, and returns what was there.
    fn cut_out(&mut self, from: Pos, to: Pos) -> String {
        if from.line == to.line {
            return self.lines[from.line].drain(from.col..to.col).collect();
        }
        let mut out = self.lines[from.line].split_off(from.col);
        let kept = self.lines[to.line].split_off(to.col);
        for line in self.lines.drain(from.line + 1..=to.line) {
            out.push('\n');
            out.push_str(&line);
        }
        self.lines[from.line].push_str(&kept);
        out
    }
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextDocument for TextBuffer {
    fn line_count(&mut self) -> Option<usize> {
        Some(self.lines.len())
    }

    fn known_lines(&mut self) -> usize {
        self.lines.len()
    }

    fn line(&mut self, n: usize) -> Option<Cow<'_, str>> {
        self.lines.get(n).map(|l| Cow::Borrowed(l.as_str()))
    }

    fn insert(&mut self, at: Pos, text: &str) {
        if text.is_empty() {
            return;
        }
        self.splice_in(at, text);
        self.redo.clear();
        // Typing along one line extends the last entry rather than making one
        // per key, so undo takes back a word rather than a letter.
        if let Some(last) = self.undo.last_mut()
            && last.inserted
            && !last.text.contains('\n')
            && !text.contains('\n')
            && end_of(last.at, &last.text) == at
        {
            last.text.push_str(text);
            return;
        }
        self.undo.push(Edit {
            at,
            text: text.to_string(),
            inserted: true,
        });
    }

    fn delete(&mut self, from: Pos, to: Pos) {
        if from >= to {
            return;
        }
        let text = self.cut_out(from, to);
        self.redo.clear();
        self.undo.push(Edit {
            at: from,
            text,
            inserted: false,
        });
    }

    fn undo(&mut self) -> bool {
        let Some(edit) = self.undo.pop() else {
            return false;
        };
        if edit.inserted {
            self.cut_out(edit.at, end_of(edit.at, &edit.text));
        } else {
            self.splice_in(edit.at, &edit.text);
        }
        self.redo.push(edit);
        true
    }

    fn redo(&mut self) -> bool {
        let Some(edit) = self.redo.pop() else {
            return false;
        };
        if edit.inserted {
            self.splice_in(edit.at, &edit.text);
        } else {
            self.cut_out(edit.at, end_of(edit.at, &edit.text));
        }
        self.undo.push(edit);
        true
    }
}

/// What the widget asks of the application's clipboard.
///
/// The widget has no clipboard to reach: this crate runs on panels with no
/// window system as well as desktops with one. So it says what it wants and
/// the application, which knows what it is running on, does it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardRequest {
    /// The selection, to put on the clipboard.
    Copy(String),
    /// The selection, already deleted from the document, to put on the
    /// clipboard.
    Cut(String),
    /// The clipboard's text is wanted: answer with [`TextArea::insert_text`].
    Paste,
}

/// Lines of text somebody edits, drawn from a [`TextDocument`].
///
/// # Blinking
///
/// As [`TextInput`](super::TextInput): the caret blinks only while the widget
/// has focus, and an unfocused editor asks for no frames.
///
/// # Focus
///
/// No focus ring. The caret is the sign that the keyboard goes here, and a
/// ring around a widget the size of a window would frame the whole window.
pub struct TextArea<M, D = TextBuffer> {
    doc: RefCell<D>,
    caret: Pos,
    /// Where the selection started, if there is one. The selection is
    /// `anchor..caret` in whichever order they fall.
    anchor: Option<Pos>,
    /// The x the caret is trying to keep while moving up and down, so a walk
    /// through short lines comes back out at the column it went in at.
    goal_x: Option<i32>,
    /// First line drawn.
    top: usize,
    /// Pixels the text is scrolled sideways. A `Cell`, because a range
    /// selected from outside an event — [`select_range`](Self::select_range)
    /// — can only be measured, and so scrolled to, by the next paint.
    scroll_x: Cell<i32>,
    /// Paint is to scroll the caret's range into view sideways.
    reveal_pending: Cell<bool>,
    /// Wheel pixels not yet worth a whole line.
    wheel_rest: i32,
    style: TextStyle,
    gutter: bool,
    /// Columns from one tab stop to the next, a column being a space wide.
    tab_width: u8,
    read_only: bool,
    on_change: Option<M>,
    on_clipboard: Option<fn(ClipboardRequest) -> M>,
    dragging: bool,
    /// The scrollbar's thumb is held: the pointer's y when it was taken, and
    /// the top line then.
    thumb_drag: Option<(i32, usize)>,
    /// Whole rows the last paint had room for, so a jump made from outside
    /// an event — [`go_to`](Self::go_to) — can centre its line.
    rows_seen: Cell<usize>,
    last_click: Option<(Pos, u64)>,
    blink_epoch: u64,
    caret_on: bool,
    has_focus: bool,
}

/// Where the scrollbar's thumb sits on a track `track_h` tall, as
/// `(offset, height)`: as tall a share of the track as the rows are of the
/// lines, never under `min_h`, and as far down as `top` is through the range
/// of tops that keep the last line on screen.
fn thumb_span(track_h: i32, rows: usize, total: usize, top: usize, min_h: i32) -> (i32, i32) {
    let total = total.max(1) as i64;
    let rows = rows.max(1) as i64;
    let h = ((track_h as i64 * rows / total) as i32)
        .max(min_h)
        .min(track_h.max(1));
    let max_top = (total - rows).max(0);
    let y = if max_top == 0 {
        0
    } else {
        ((track_h - h) as i64 * (top as i64).min(max_top) / max_top) as i32
    };
    (y, h)
}

impl<M> TextArea<M, TextBuffer> {
    /// An editor over a buffer holding `text`.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        Self::new(TextBuffer::from_text(text))
    }

    /// The whole text of the buffer.
    #[must_use]
    pub fn text(&self) -> String {
        self.doc.borrow().text()
    }

    /// Replaces the text, putting the caret at the start. Silent, and the
    /// undo history goes with the old text.
    pub fn set_text(&mut self, text: &str) {
        *self.doc.get_mut() = TextBuffer::from_text(text);
        self.caret = Pos::ZERO;
        self.anchor = None;
        self.top = 0;
        self.scroll_x.set(0);
    }
}

impl<M> Default for TextArea<M, TextBuffer> {
    fn default() -> Self {
        Self::from_text("")
    }
}

impl<M, D: TextDocument> TextArea<M, D> {
    /// An editor over `doc`.
    #[must_use]
    pub fn new(doc: D) -> Self {
        Self {
            doc: RefCell::new(doc),
            caret: Pos::ZERO,
            anchor: None,
            goal_x: None,
            top: 0,
            scroll_x: Cell::new(0),
            reveal_pending: Cell::new(false),
            wheel_rest: 0,
            style: TextStyle::built_in(16),
            gutter: true,
            tab_width: 4,
            read_only: false,
            on_change: None,
            on_clipboard: None,
            dragging: false,
            thumb_drag: None,
            rows_seen: Cell::new(0),
            last_click: None,
            blink_epoch: 0,
            caret_on: true,
            has_focus: false,
        }
    }

    /// Sets the message emitted after every edit a person makes.
    #[must_use]
    pub fn with_change(mut self, message: M) -> Self {
        self.on_change = Some(message);
        self
    }

    /// Sets how clipboard requests reach the application. Without it, copy,
    /// cut and paste do nothing.
    #[must_use]
    pub fn with_clipboard(mut self, message: fn(ClipboardRequest) -> M) -> Self {
        self.on_clipboard = Some(message);
        self
    }

    /// Sets the font and size.
    #[must_use]
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets the size, keeping the font.
    #[must_use]
    pub fn with_size(mut self, size_px: u16) -> Self {
        self.style.size_px = size_px;
        self
    }

    /// Whether to number the lines down the left.
    #[must_use]
    pub fn with_gutter(mut self, gutter: bool) -> Self {
        self.gutter = gutter;
        self
    }

    /// Sets how many columns apart the tab stops are; four unless told.
    /// A column is a space wide, and at least one.
    #[must_use]
    pub fn with_tab_width(mut self, columns: u8) -> Self {
        self.tab_width = columns.max(1);
        self
    }

    /// Shows the text and places a caret in it, but changes nothing.
    #[must_use]
    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// The document.
    pub fn document(&self) -> core::cell::Ref<'_, D> {
        self.doc.borrow()
    }

    /// The document, to change. Silent; the caret is clamped to whatever the
    /// document says next time it is used.
    pub fn document_mut(&mut self) -> &mut D {
        self.doc.get_mut()
    }

    /// Replaces the document, putting the caret at the start.
    pub fn set_document(&mut self, doc: D) {
        *self.doc.get_mut() = doc;
        self.caret = Pos::ZERO;
        self.anchor = None;
        self.top = 0;
        self.scroll_x.set(0);
    }

    /// The font and size.
    #[inline]
    pub const fn style(&self) -> TextStyle {
        self.style
    }

    /// Replaces the font and size.
    pub fn set_style(&mut self, style: TextStyle) {
        self.style = style;
    }

    /// Whether the lines are numbered.
    #[inline]
    pub const fn gutter(&self) -> bool {
        self.gutter
    }

    /// Numbers the lines, or stops.
    pub fn set_gutter(&mut self, gutter: bool) {
        self.gutter = gutter;
    }

    /// How many columns apart the tab stops are.
    #[inline]
    pub const fn tab_width(&self) -> u8 {
        self.tab_width
    }

    /// Moves the tab stops `columns` apart, at least one.
    pub fn set_tab_width(&mut self, columns: u8) {
        self.tab_width = columns.max(1);
    }

    /// Whether editing is off.
    #[inline]
    pub const fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Turns editing off or on.
    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }

    /// Where the caret is.
    #[inline]
    pub const fn caret(&self) -> Pos {
        self.caret
    }

    /// Puts the caret at `pos`, clamped to the document, and clears the
    /// selection. The view follows on the next event or paint.
    pub fn set_caret(&mut self, pos: Pos) {
        self.caret = self.clamp(pos);
        self.anchor = None;
        self.goal_x = None;
    }

    /// The selection as `(from, to)`, or `None` if nothing is selected.
    pub fn selection(&self) -> Option<(Pos, Pos)> {
        let anchor = self.anchor.filter(|a| *a != self.caret)?;
        Some((anchor.min(self.caret), anchor.max(self.caret)))
    }

    /// Selects everything.
    pub fn select_all(&mut self) {
        self.anchor = Some(Pos::ZERO);
        self.caret = self.last_pos();
        self.goal_x = None;
    }

    /// The selected text, lines joined by `\n`, or `None` if nothing is
    /// selected.
    pub fn selected_text(&self) -> Option<String> {
        let (from, to) = self.selection()?;
        let mut doc = self.doc();
        let mut out = String::new();
        for n in from.line..=to.line {
            let Some(line) = doc.line(n) else { break };
            let start = if n == from.line { from.col } else { 0 };
            let end = if n == to.line { to.col } else { line.len() };
            if n != from.line {
                out.push('\n');
            }
            out.push_str(&line[start.min(end)..end]);
        }
        Some(out)
    }

    /// Replaces the selection, or inserts at the caret, and leaves the caret
    /// after the text. Silent: this is how an application answers a
    /// [`ClipboardRequest::Paste`], and it knows it did.
    pub fn insert_text(&mut self, text: &str) {
        self.replace_selection(text);
    }

    /// First line drawn.
    #[inline]
    pub const fn top(&self) -> usize {
        self.top
    }

    /// Scrolls so `line` is the first drawn, clamped to the lines known.
    pub fn set_top(&mut self, line: usize) {
        let last = self.known_lines() - 1;
        self.top = line.min(last);
    }

    /// Whole rows the last paint had room for; 0 before the first.
    pub fn visible_rows(&self) -> usize {
        self.rows_seen.get()
    }

    /// Puts the caret at the start of 0-based `line`, clamped to the lines
    /// known, and scrolls so the line sits in the middle of the view — as
    /// far as the last paint's row count can say where the middle is.
    pub fn go_to(&mut self, line: usize) {
        self.set_caret(Pos::new(line, 0));
        let rows = self.rows_seen.get();
        let top = self.caret.line.saturating_sub(rows / 2);
        self.top = top.min(self.max_top(rows));
        self.scroll_x.set(0);
    }

    /// Selects `[from, to)` with the caret at `to`, both clamped to the
    /// document, and scrolls the selection into view: centred on its first
    /// line if that was off screen, and sideways on the next paint, which has
    /// the fonts to measure it with. What a find lands on. Silent.
    pub fn select_range(&mut self, from: Pos, to: Pos) {
        let (from, to) = (self.clamp(from), self.clamp(to));
        self.anchor = Some(from);
        self.caret = to;
        self.goal_x = None;
        let rows = self.rows_seen.get();
        let shown = rows > 0 && from.line >= self.top && to.line < self.top + rows;
        if !shown {
            let top = from.line.saturating_sub(rows / 2);
            self.top = top.min(self.max_top(rows));
        }
        self.reveal_pending.set(true);
    }

    /// Pixels the text is scrolled sideways.
    pub fn scroll_x(&self) -> i32 {
        self.scroll_x.get()
    }

    /// The largest top that still fills `rows` rows, or the last line when
    /// there are fewer lines than that.
    fn max_top(&self, rows: usize) -> usize {
        self.known_lines().saturating_sub(rows.max(1))
    }

    fn doc(&self) -> RefMut<'_, D> {
        self.doc.borrow_mut()
    }

    /// Line `n` as its own `String`, so nothing holds the document while the
    /// text is used.
    fn line_text(&self, n: usize) -> Option<String> {
        self.doc().line(n).map(Cow::into_owned)
    }

    fn line_len(&self, n: usize) -> usize {
        self.doc().line(n).map_or(0, |l| l.len())
    }

    fn known_lines(&self) -> usize {
        self.doc().known_lines().max(1)
    }

    /// The end of the last known line.
    fn last_pos(&self) -> Pos {
        let line = self.known_lines() - 1;
        Pos::new(line, self.line_len(line))
    }

    /// `pos` moved onto a line and a character boundary that exist.
    fn clamp(&self, pos: Pos) -> Pos {
        let line = pos.line.min(self.known_lines() - 1);
        let Some(text) = self.line_text(line) else {
            return Pos::new(line, 0);
        };
        let mut col = pos.col.min(text.len());
        while !text.is_char_boundary(col) {
            col -= 1;
        }
        Pos::new(line, col)
    }

    // ---- geometry ---------------------------------------------------------

    /// Padding between the gutter and the text, and inside the gutter.
    #[inline]
    const fn pad(&self) -> i32 {
        self.style.size_px as i32 / 3
    }

    fn row_height(&self, engine: &TextEngine) -> i32 {
        engine.line_height(self.style).max(1)
    }

    /// Width of the gutter, numbers or not.
    fn gutter_width(&self, engine: &mut TextEngine) -> i32 {
        if !self.gutter {
            return self.pad();
        }
        let mut doc = self.doc();
        let count = doc.line_count().unwrap_or_else(|| doc.known_lines()).max(1);
        drop(doc);
        let digits = count.to_string().len().max(3) as i32;
        engine.measure_line(self.style, "0").max(1) * digits + self.pad() * 2
    }

    /// Width of the scrollbar strip down the right.
    #[inline]
    fn bar_width(&self) -> i32 {
        (self.style.size_px as i32 * 5 / 8).max(6)
    }

    /// The scrollbar's strip.
    fn bar_rect(&self, bounds: Rect) -> Rect {
        let w = self.bar_width().min(bounds.width.max(0));
        Rect::new(bounds.right() - w, bounds.y, w, bounds.height)
    }

    /// The thumb within the strip, or `None` when everything fits.
    fn thumb_rect(&self, engine: &TextEngine, bounds: Rect) -> Option<Rect> {
        let rows = self.rows(engine, bounds);
        let total = self.known_lines();
        if total <= rows {
            return None;
        }
        let bar = self.bar_rect(bounds);
        // A pixel of strip either side of the thumb, and two above and
        // below: wide enough to find and grab, with the strip still showing
        // as its track.
        let (side, end) = (1, 2);
        let track_h = (bar.height - end * 2).max(1);
        let (y, h) = thumb_span(track_h, rows, total, self.top, self.bar_width() * 2);
        Some(Rect::new(
            bar.x + side,
            bar.y + end + y,
            (bar.width - side * 2).max(1),
            h,
        ))
    }

    /// The rectangle the text is drawn in: between the gutter and the
    /// scrollbar.
    fn text_rect(&self, engine: &mut TextEngine, bounds: Rect) -> Rect {
        let gutter = self.gutter_width(engine);
        let bar = self.bar_width();
        Rect::new(
            bounds.x + gutter,
            bounds.y,
            (bounds.width - gutter - bar).max(0),
            bounds.height,
        )
    }

    /// Whole rows the bounds hold.
    fn rows(&self, engine: &TextEngine, bounds: Rect) -> usize {
        (bounds.height / self.row_height(engine)).max(1) as usize
    }

    /// Horizontal offset of `col` within `line`, unscrolled, with every tab
    /// before it reaching its stop.
    ///
    /// Everything that turns a column into a position goes through here or
    /// through [`advance`](Self::advance) — the caret, a click, the
    /// selection, the marks, scrolling to the caret — so a tab is as wide to
    /// all of them as it is drawn.
    fn x_of(&self, engine: &mut TextEngine, line: &str, col: usize) -> i32 {
        self.advance(engine, 0, &line[..col.min(line.len())])
    }

    /// Where `text` ends when it starts `x` pixels into its line: its width
    /// added, except that a tab jumps to the next stop. Stops are counted
    /// from the start of the line, so `x` must be too.
    fn advance(&self, engine: &mut TextEngine, mut x: i32, text: &str) -> i32 {
        for (i, piece) in text.split('\t').enumerate() {
            if i > 0 {
                x = self.next_stop(engine, x);
            }
            if !piece.is_empty() {
                x += engine.measure_line(self.style, piece);
            }
        }
        x
    }

    /// The first tab stop past `x`: a tab at a stop goes on to the next one,
    /// as it does in every terminal.
    fn next_stop(&self, engine: &mut TextEngine, x: i32) -> i32 {
        let stop = engine.measure_line(self.style, " ").max(1) * i32::from(self.tab_width.max(1));
        (x.max(0) / stop + 1) * stop
    }

    /// Draws `text` starting `x` pixels into the line that begins at
    /// `origin`, tabs reaching their stops, and returns where it ends.
    fn draw_run(
        &self,
        engine: &mut TextEngine,
        canvas: &mut Pen<'_>,
        origin: Point,
        mut x: i32,
        text: &str,
        color: Color,
    ) -> i32 {
        for (i, piece) in text.split('\t').enumerate() {
            if i > 0 {
                x = self.next_stop(engine, x);
            }
            if !piece.is_empty() {
                let at = Point::new(origin.x + x, origin.y);
                x += engine.draw(canvas, self.style, at, piece, color).width as i32;
            }
        }
        x
    }

    /// The character boundary of `line` nearest to `x`.
    fn col_at_x(&self, engine: &mut TextEngine, line: &str, x: i32) -> usize {
        if x <= 0 || line.is_empty() {
            return 0;
        }
        let bounds: Vec<usize> = line
            .char_indices()
            .map(|(i, _)| i)
            .chain(core::iter::once(line.len()))
            .collect();
        // The last boundary whose prefix fits, found by bisection: text only
        // gets wider as it gets longer.
        let (mut lo, mut hi) = (0, bounds.len() - 1);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if self.x_of(engine, line, bounds[mid]) <= x {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        // Past the middle of the character is the far side of it.
        if lo + 1 < bounds.len() {
            let here = self.x_of(engine, line, bounds[lo]);
            let next = self.x_of(engine, line, bounds[lo + 1]);
            if x - here > next - x {
                return bounds[lo + 1];
            }
        }
        bounds[lo]
    }

    /// The position under `point`.
    fn pos_at(&self, engine: &mut TextEngine, bounds: Rect, point: Point) -> Pos {
        let row_h = self.row_height(engine);
        let row = ((point.y - bounds.y).max(0) / row_h).max(0) as usize;
        let line = (self.top + row).min(self.known_lines() - 1);
        let text = self.line_text(line).unwrap_or_default();
        let x = point.x - self.text_rect(engine, bounds).x + self.scroll_x.get();
        Pos::new(line, self.col_at_x(engine, &text, x))
    }

    /// Scrolls so the caret is on screen.
    fn reveal_caret(&mut self, engine: &mut TextEngine, bounds: Rect) {
        let rows = self.rows(engine, bounds);
        if self.caret.line < self.top {
            self.top = self.caret.line;
        } else if self.caret.line >= self.top + rows {
            self.top = self.caret.line + 1 - rows;
        }
        self.reveal_caret_x(engine, bounds);
    }

    /// Scrolls sideways so the caret is on screen.
    fn reveal_caret_x(&self, engine: &mut TextEngine, bounds: Rect) {
        let area = self.text_rect(engine, bounds);
        if area.width <= 0 {
            return;
        }
        let Some(line) = self.line_text(self.caret.line) else {
            return;
        };
        let x = self.x_of(engine, &line, self.caret.col);
        let caret_w = self.caret_width() + self.pad();
        let scroll = self.scroll_x.get();
        if x < scroll {
            self.scroll_x.set(x);
        } else if x + caret_w > scroll + area.width {
            self.scroll_x.set(x + caret_w - area.width);
        }
    }

    /// Scrolls sideways so the caret is on screen and, when the selection is
    /// on one line and fits, the start of it too: a match at the far end of a
    /// long line is shown whole rather than cut at the left edge.
    fn reveal_range_x(&self, engine: &mut TextEngine, bounds: Rect) {
        self.reveal_caret_x(engine, bounds);
        let Some(anchor) = self.anchor else { return };
        if anchor.line != self.caret.line || anchor.col >= self.caret.col {
            return;
        }
        let Some(line) = self.line_text(self.caret.line) else {
            return;
        };
        let width = self.text_rect(engine, bounds).width;
        let start = self.x_of(engine, &line, anchor.col);
        let end = self.x_of(engine, &line, self.caret.col) + self.caret_width() + self.pad();
        if start < self.scroll_x.get() && end - start <= width {
            self.scroll_x.set(start);
        }
    }

    #[inline]
    fn caret_width(&self) -> i32 {
        (i32::from(self.style.size_px) / 10).max(1)
    }

    // ---- moving -----------------------------------------------------------

    /// Moves the caret, extending the selection or dropping it.
    fn move_to(&mut self, to: Pos, extend: bool) {
        if extend {
            self.anchor.get_or_insert(self.caret);
        } else {
            self.anchor = None;
        }
        self.caret = to;
    }

    fn left_of(&self, pos: Pos) -> Pos {
        if pos.col > 0 {
            let line = self.line_text(pos.line).unwrap_or_default();
            Pos::new(pos.line, prev_boundary(&line, pos.col))
        } else if pos.line > 0 {
            Pos::new(pos.line - 1, self.line_len(pos.line - 1))
        } else {
            pos
        }
    }

    fn right_of(&self, pos: Pos) -> Pos {
        let line = self.line_text(pos.line).unwrap_or_default();
        if pos.col < line.len() {
            Pos::new(pos.line, next_boundary(&line, pos.col))
        } else if pos.line + 1 < self.known_lines() {
            Pos::new(pos.line + 1, 0)
        } else {
            pos
        }
    }

    /// The caret `by` lines away, at the x it is trying to keep.
    fn vertical(&mut self, engine: &mut TextEngine, by: isize) -> Pos {
        let goal = match self.goal_x {
            Some(x) => x,
            None => {
                let line = self.line_text(self.caret.line).unwrap_or_default();
                let x = self.x_of(engine, &line, self.caret.col);
                self.goal_x = Some(x);
                x
            }
        };
        let last = self.known_lines() - 1;
        let line = self.caret.line.saturating_add_signed(by).min(last);
        let text = self.line_text(line).unwrap_or_default();
        Pos::new(line, self.col_at_x(engine, &text, goal))
    }

    // ---- editing ----------------------------------------------------------

    /// Deletes the selection if there is one and inserts `text` at the caret.
    fn replace_selection(&mut self, text: &str) {
        if let Some((from, to)) = self.selection() {
            self.doc().delete(from, to);
            self.caret = from;
        }
        self.anchor = None;
        self.goal_x = None;
        if !text.is_empty() {
            self.doc().insert(self.caret, text);
            self.caret = end_of(self.caret, text);
        }
    }

    fn backspace(&mut self) -> bool {
        if self.selection().is_some() {
            self.replace_selection("");
            return true;
        }
        let from = self.left_of(self.caret);
        if from == self.caret {
            return false;
        }
        self.doc().delete(from, self.caret);
        self.caret = from;
        self.goal_x = None;
        true
    }

    fn delete_forward(&mut self) -> bool {
        if self.selection().is_some() {
            self.replace_selection("");
            return true;
        }
        let to = self.right_of(self.caret);
        if to == self.caret {
            return false;
        }
        self.doc().delete(self.caret, to);
        self.goal_x = None;
        true
    }

    /// Restarts the blink so the caret is solid while it is being moved.
    fn wake_caret(&mut self, now_ms: u64) {
        self.blink_epoch = now_ms;
        self.caret_on = true;
    }

    /// What every caret move ends with.
    fn moved(&mut self, ctx: &mut EventCtx<'_, M>) -> Handled {
        self.wake_caret(ctx.now_ms);
        let bounds = ctx.bounds;
        self.reveal_caret(ctx.text, bounds);
        Handled::Yes
    }

    fn key(&mut self, code: KeyCode, modifiers: Modifiers, ctx: &mut EventCtx<'_, M>) -> Handled
    where
        M: Clone,
    {
        let shift = modifiers.contains(Modifiers::SHIFT);
        // Ctrl on the desktops that use it, Command on the one that does not;
        // a panel with a bare keyboard has neither and needs neither.
        let primary = modifiers.contains(Modifiers::CTRL) || modifiers.contains(Modifiers::SUPER);
        let edited = match code {
            KeyCode::ArrowLeft => {
                let to = match self.selection() {
                    Some((from, _)) if !shift => from,
                    _ => self.left_of(self.caret),
                };
                self.move_to(to, shift);
                self.goal_x = None;
                return self.moved(ctx);
            }
            KeyCode::ArrowRight => {
                let to = match self.selection() {
                    Some((_, to)) if !shift => to,
                    _ => self.right_of(self.caret),
                };
                self.move_to(to, shift);
                self.goal_x = None;
                return self.moved(ctx);
            }
            KeyCode::ArrowUp | KeyCode::ArrowDown => {
                let by = if code == KeyCode::ArrowUp { -1 } else { 1 };
                let to = self.vertical(ctx.text, by);
                self.move_to(to, shift);
                return self.moved(ctx);
            }
            KeyCode::PageUp | KeyCode::PageDown => {
                let rows = self.rows(ctx.text, ctx.bounds) as isize;
                let by = if code == KeyCode::PageUp { -rows } else { rows };
                let to = self.vertical(ctx.text, by);
                self.move_to(to, shift);
                // The view pages with the caret rather than merely following
                // it, so the caret keeps its row on screen.
                let max_top = self.max_top(rows as usize);
                self.top = self.top.saturating_add_signed(by).min(max_top);
                return self.moved(ctx);
            }
            KeyCode::Home => {
                let to = if primary {
                    Pos::ZERO
                } else {
                    Pos::new(self.caret.line, 0)
                };
                self.move_to(to, shift);
                self.goal_x = None;
                return self.moved(ctx);
            }
            KeyCode::End => {
                let to = if primary {
                    self.last_pos()
                } else {
                    Pos::new(self.caret.line, self.line_len(self.caret.line))
                };
                self.move_to(to, shift);
                self.goal_x = None;
                return self.moved(ctx);
            }
            KeyCode::Escape => {
                if self.anchor.take().is_none() {
                    return Handled::No;
                }
                return Handled::Yes;
            }
            KeyCode::A if primary => {
                self.select_all();
                return self.moved(ctx);
            }
            KeyCode::C if primary => {
                return self.clipboard(ctx, false);
            }
            KeyCode::X if primary => {
                return self.clipboard(ctx, true);
            }
            KeyCode::V if primary => {
                if self.read_only {
                    return Handled::No;
                }
                let Some(request) = self.on_clipboard else {
                    return Handled::No;
                };
                ctx.emit(request(ClipboardRequest::Paste));
                return Handled::Yes;
            }
            KeyCode::Z if primary && !self.read_only => {
                let done = if shift {
                    self.doc().redo()
                } else {
                    self.doc().undo()
                };
                if !done {
                    return Handled::No;
                }
                // The document moved under the caret and only it knows where
                // to; the nearest place that still exists is the honest guess.
                self.caret = self.clamp(self.caret);
                self.anchor = None;
                self.goal_x = None;
                true
            }
            KeyCode::Y if primary && !self.read_only => {
                if !self.doc().redo() {
                    return Handled::No;
                }
                self.caret = self.clamp(self.caret);
                self.anchor = None;
                self.goal_x = None;
                true
            }
            KeyCode::Backspace if !self.read_only => self.backspace(),
            KeyCode::Delete if !self.read_only => self.delete_forward(),
            KeyCode::Enter | KeyCode::NumpadEnter if !self.read_only => {
                self.replace_selection("\n");
                true
            }
            _ => return Handled::No,
        };
        if edited && let Some(message) = self.on_change.clone() {
            ctx.emit(message);
        }
        self.moved(ctx)
    }

    /// Copy, or cut, through the application.
    fn clipboard(&mut self, ctx: &mut EventCtx<'_, M>, cut: bool) -> Handled
    where
        M: Clone,
    {
        let Some(request) = self.on_clipboard else {
            return Handled::No;
        };
        let Some(text) = self.selected_text() else {
            return Handled::No;
        };
        if cut && !self.read_only {
            self.replace_selection("");
            ctx.emit(request(ClipboardRequest::Cut(text)));
            if let Some(message) = self.on_change.clone() {
                ctx.emit(message);
            }
            return self.moved(ctx);
        }
        ctx.emit(request(ClipboardRequest::Copy(text)));
        Handled::Yes
    }

    fn press(
        &mut self,
        position: Point,
        modifiers: Modifiers,
        ctx: &mut EventCtx<'_, M>,
    ) -> Handled {
        let bounds = ctx.bounds;
        if self.bar_rect(bounds).contains(position) {
            return self.press_bar(position, ctx);
        }
        let pos = self.pos_at(ctx.text, bounds, position);
        let now = ctx.now_ms;
        let again = self
            .last_click
            .is_some_and(|(at, when)| at == pos && now.saturating_sub(when) <= DOUBLE_CLICK_MS);
        if again {
            // A second click on the same spot takes the word under it.
            let line = self.line_text(pos.line).unwrap_or_default();
            let (start, end) = word_at(&line, pos.col);
            self.anchor = Some(Pos::new(pos.line, start));
            self.caret = Pos::new(pos.line, end);
            self.last_click = None;
            self.dragging = false;
        } else {
            self.move_to(pos, modifiers.contains(Modifiers::SHIFT));
            self.last_click = Some((pos, now));
            self.dragging = true;
        }
        self.goal_x = None;
        self.moved(ctx)
    }

    /// A press in the scrollbar: takes the thumb, or pages towards the press.
    fn press_bar(&mut self, position: Point, ctx: &mut EventCtx<'_, M>) -> Handled {
        let bounds = ctx.bounds;
        let Some(thumb) = self.thumb_rect(ctx.text, bounds) else {
            return Handled::No;
        };
        let rows = self.rows(ctx.text, bounds);
        if thumb.contains(position) {
            self.thumb_drag = Some((position.y, self.top));
        } else if position.y < thumb.y {
            self.top = self.top.saturating_sub(rows);
        } else {
            self.top = (self.top + rows).min(self.max_top(rows));
        }
        Handled::Yes
    }

    /// The thumb, held, followed the pointer to `y`.
    fn drag_thumb(&mut self, engine: &TextEngine, bounds: Rect, y: i32) -> Handled {
        let Some((from_y, from_top)) = self.thumb_drag else {
            return Handled::No;
        };
        let Some(thumb) = self.thumb_rect(engine, bounds) else {
            return Handled::No;
        };
        let rows = self.rows(engine, bounds);
        let max_top = self.max_top(rows);
        let travel = (self.bar_rect(bounds).height - 4 - thumb.height).max(1) as i64;
        let moved = (y - from_y) as i64 * max_top as i64 / travel;
        let top = (from_top as i64 + moved).clamp(0, max_top as i64) as usize;
        if top == self.top {
            return Handled::No;
        }
        self.top = top;
        Handled::Yes
    }

    fn wheel(&mut self, engine: &TextEngine, bounds: Rect, delta_x: f32, delta_y: f32) -> Handled {
        let row_h = self.row_height(engine);
        self.wheel_rest += delta_y as i32;
        let lines = self.wheel_rest / row_h;
        self.wheel_rest -= lines * row_h;
        let max_top = self.max_top(self.rows(engine, bounds));
        let top = self.top.saturating_add_signed(lines as isize).min(max_top);
        let scroll_x = (self.scroll_x.get() + delta_x as i32).max(0);
        if top == self.top && scroll_x == self.scroll_x.get() {
            return Handled::No;
        }
        self.top = top;
        self.scroll_x.set(scroll_x);
        Handled::Yes
    }
}

/// The boundary before `col` in `line`.
fn prev_boundary(line: &str, col: usize) -> usize {
    line[..col]
        .chars()
        .next_back()
        .map_or(0, |c| col - c.len_utf8())
}

/// The boundary after `col` in `line`.
fn next_boundary(line: &str, col: usize) -> usize {
    line[col..]
        .chars()
        .next()
        .map_or(col, |c| col + c.len_utf8())
}

/// Whether `c` is part of a word, for double-click selection.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The run of word characters — or of non-word ones — around `col`.
fn word_at(line: &str, col: usize) -> (usize, usize) {
    // The word before wins at its end, so a double-click just past the last
    // letter takes the word rather than the space after it.
    let before = line[..col].chars().next_back();
    let after = line[col..].chars().next();
    let class = match (before, after) {
        (Some(b), _) if is_word(b) => true,
        (_, Some(a)) => is_word(a),
        (Some(b), None) => is_word(b),
        (None, None) => return (col, col),
    };
    let start = line[..col]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word(*c) == class)
        .last()
        .map_or(col, |(i, _)| i);
    let end = line[col..]
        .char_indices()
        .find(|(_, c)| is_word(*c) != class)
        .map_or(line.len(), |(i, _)| col + i);
    (start, end)
}

impl<M: Clone + 'static, D: TextDocument> Widget<M> for TextArea<M, D> {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }

    fn measure(&self, ctx: &mut MeasureCtx<'_>, _offered: Offer) -> Measured {
        // An editor is as big as you make it; the one thing it can say is
        // that fewer than three lines is not an editor.
        Measured::tall(ctx.text.line_height(self.style).max(1) * 3)
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        let bounds = ctx.bounds;
        let theme = ctx.theme;
        let disabled = ctx.state.contains(VisualState::DISABLED);
        let focused = ctx.state.contains(VisualState::FOCUSED);
        canvas.fill_rect(bounds, theme.color(Role::Base100));

        let row_h = self.row_height(ctx.text);
        let gutter_w = self.gutter_width(ctx.text);
        if self.reveal_pending.take() {
            self.reveal_range_x(ctx.text, bounds);
        }
        let area = self.text_rect(ctx.text, bounds);
        let text_x = area.x - self.scroll_x.get();
        let content = if disabled {
            theme.color(Role::Base300)
        } else {
            theme.color(Role::BaseContent)
        };
        let dim = muted(theme.color(Role::Base100), content);
        let selected = theme.color(Role::Accent).with_alpha(60);
        let marked = theme.color(Role::Warning).with_alpha(90);
        let space_w = ctx.text.measure_line(self.style, " ").max(1);
        let selection = self.selection();

        if self.gutter {
            let gutter = Rect::new(bounds.x, bounds.y, gutter_w, bounds.height);
            canvas.fill_rect(gutter, theme.color(Role::Base200));
        }

        self.rows_seen.set(self.rows(ctx.text, bounds));
        let mut spans = Vec::new();
        let mut marks: Vec<Range<usize>> = Vec::new();
        let rows = (bounds.height / row_h + 1).max(1) as usize;
        for row in 0..rows {
            let n = self.top + row;
            let Some(line) = self.line_text(n) else { break };
            let y = bounds.y + row as i32 * row_h;
            if y >= bounds.bottom() {
                break;
            }
            let strip = Rect::new(bounds.x, y, bounds.width, row_h);
            if !strip.intersects(&canvas.clip()) {
                continue;
            }

            if self.gutter {
                let number = (n + 1).to_string();
                let w = ctx.text.measure_line(self.style, &number);
                let x = bounds.x + gutter_w - self.pad() - w;
                let color = if n == self.caret.line { content } else { dim };
                ctx.text
                    .draw(canvas, self.style, Point::new(x, y), &number, color);
            }

            let mut clipped = canvas.with_clip(area);
            marks.clear();
            if !disabled {
                self.doc().highlights(n, &line, &mut marks);
            }
            // Measured a stretch at a time from the last mark's end, rather
            // than each from the start of the line: a one-letter search on a
            // long line is thousands of marks, and measuring every prefix
            // would be quadratic in the line. Marks past the right edge are
            // not measured at all.
            // Positions are kept from the start of the line, where tab stops
            // are counted from, and moved by the scroll only to be drawn.
            let (mut col, mut x) = (0, 0);
            for mark in &marks {
                if mark.start < col
                    || mark.start >= mark.end
                    || mark.end > line.len()
                    || !line.is_char_boundary(mark.start)
                    || !line.is_char_boundary(mark.end)
                {
                    continue;
                }
                let start = self.advance(ctx.text, x, &line[col..mark.start]);
                if text_x + start >= area.right() {
                    break;
                }
                let end = self.advance(ctx.text, start, &line[mark.start..mark.end]);
                if text_x + end > area.x {
                    clipped.fill_rect(Rect::new(text_x + start, y, end - start, row_h), marked);
                }
                (col, x) = (mark.end, end);
            }

            if let Some((from, to)) = selection
                && n >= from.line
                && n <= to.line
            {
                let start = if n == from.line { from.col } else { 0 };
                let end = if n == to.line { to.col } else { line.len() };
                let x0 = text_x + self.x_of(ctx.text, &line, start);
                let mut x1 = text_x + self.x_of(ctx.text, &line, end);
                if n < to.line {
                    // The newline is selected too, and is a space wide.
                    x1 += space_w;
                }
                clipped.fill_rect(Rect::new(x0, y, x1 - x0, row_h), selected);
            }

            spans.clear();
            if !disabled {
                self.doc().spans(n, &mut spans);
            }
            let origin = Point::new(text_x, y);
            let mut x = 0;
            let mut at = 0;
            for span in spans
                .iter()
                .filter(|s| s.start < s.end && s.end <= line.len())
            {
                if span.start > at {
                    let run = &line[at..span.start];
                    x = self.draw_run(ctx.text, &mut clipped, origin, x, run, content);
                }
                let run = &line[span.start..span.end];
                x = self.draw_run(ctx.text, &mut clipped, origin, x, run, span.color);
                at = span.end;
            }
            if at < line.len() {
                self.draw_run(ctx.text, &mut clipped, origin, x, &line[at..], content);
            }

            if n == self.caret.line && focused && self.caret_on && !disabled {
                let x = text_x + self.x_of(ctx.text, &line, self.caret.col);
                clipped.fill_rect(
                    Rect::new(x, y, self.caret_width(), row_h),
                    theme.color(Role::Accent),
                );
            }
        }

        // The scrollbar: a strip down the right, a thumb only when there is
        // more than fits. Its share of the strip is the rows' share of the
        // lines, so a file still being counted shows a thumb that shrinks as
        // the count climbs.
        if let Some(thumb) = self.thumb_rect(ctx.text, bounds) {
            let bar = self.bar_rect(bounds);
            let radius = bar.width / 3;
            canvas.fill_rect(bar, theme.color(Role::Base200));
            let alpha = if self.thumb_drag.is_some() { 170 } else { 110 };
            canvas.fill_rounded_rect(thumb, radius, content.with_alpha(alpha));
        }
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        match event {
            Event::FocusGained => {
                self.has_focus = true;
                self.wake_caret(ctx.now_ms);
                ctx.request_animation();
                Handled::No
            }
            Event::FocusLost => {
                self.has_focus = false;
                self.dragging = false;
                self.thumb_drag = None;
                self.wake_caret(ctx.now_ms);
                Handled::No
            }
            Event::PressCancelled => {
                self.dragging = false;
                self.thumb_drag = None;
                Handled::No
            }
            Event::Input(InputEvent::Text { ch }) if !ch.is_control() => {
                if self.read_only {
                    return Handled::No;
                }
                let mut buf = [0; 4];
                self.replace_selection(ch.encode_utf8(&mut buf));
                if let Some(message) = self.on_change.clone() {
                    ctx.emit(message);
                }
                self.moved(ctx)
            }
            Event::Input(InputEvent::Key {
                code,
                state: ElementState::Down,
                modifiers,
                ..
            }) => self.key(*code, *modifiers, ctx),
            Event::Input(InputEvent::PointerButton {
                button: PointerButton::Left,
                state: ElementState::Down,
                position,
                modifiers,
            }) => self.press(*position, *modifiers, ctx),
            Event::Input(InputEvent::PointerButton {
                button: PointerButton::Left,
                state: ElementState::Up,
                ..
            }) => {
                self.dragging = false;
                let held = self.thumb_drag.take().is_some();
                // The thumb draws differently while held.
                if held { Handled::Yes } else { Handled::No }
            }
            Event::Input(InputEvent::PointerMoved { position }) if self.thumb_drag.is_some() => {
                let bounds = ctx.bounds;
                self.drag_thumb(ctx.text, bounds, position.y)
            }
            Event::Input(InputEvent::PointerMoved { position }) if self.dragging => {
                let bounds = ctx.bounds;
                let pos = self.pos_at(ctx.text, bounds, *position);
                if pos == self.caret {
                    return Handled::No;
                }
                self.move_to(pos, true);
                self.goal_x = None;
                self.moved(ctx)
            }
            Event::Input(InputEvent::PointerScroll {
                delta_x, delta_y, ..
            }) => {
                let bounds = ctx.bounds;
                self.wheel(ctx.text, bounds, *delta_x, *delta_y)
            }
            _ => Handled::No,
        }
    }

    fn accepts_pointer(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        true
    }

    fn animate(&mut self, now_ms: u64) -> Animation {
        if !self.has_focus {
            return Animation::NONE;
        }
        let elapsed = now_ms.saturating_sub(self.blink_epoch);
        let on = (elapsed / BLINK_MS).is_multiple_of(2);
        let repaint = on != self.caret_on;
        self.caret_on = on;
        Animation {
            repaint,
            next: Wake::At(
                self.blink_epoch.saturating_add(
                    (elapsed / BLINK_MS)
                        .saturating_add(1)
                        .saturating_mul(BLINK_MS),
                ),
            ),
        }
    }

    /// Blinking is a schedule, not a motion; see [`TextInput`](super::TextInput).
    fn snap(&mut self, now_ms: u64) -> Animation {
        Widget::<M>::animate(self, now_ms)
    }
}

impl<M, D: TextDocument> Describe for TextArea<M, D> {
    const KIND: &'static str = "text-area";
    const DOC: &'static str = "Lines of text somebody edits.";
    const GROUP: Group = Group::Input;
    const ICON: &'static denise::icon::Icon = &super::icons::TEXT_AREA;

    const PROPERTIES: &'static [Property] = &[
        Property::new(
            "gutter",
            PropertyKind::Bool,
            "Number the lines down the left.",
        ),
        Property::new(
            "read-only",
            PropertyKind::Bool,
            "Show the text and place a caret in it, but change nothing.",
        ),
        Property::new(
            "size",
            PropertyKind::Int { min: 6, max: 96 },
            "Text size in logical pixels.",
        )
        .in_pixels(),
        Property::new(
            "tab-width",
            PropertyKind::Int { min: 1, max: 16 },
            "Columns from one tab stop to the next.",
        ),
    ];

    fn get(&self, name: &str) -> Option<Value> {
        Some(match name {
            "gutter" => Value::Bool(self.gutter),
            "read-only" => Value::Bool(self.read_only),
            "size" => Value::Int(i32::from(self.style.size_px)),
            "tab-width" => Value::Int(i32::from(self.tab_width)),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            "gutter" => self.gutter = value.as_bool()?,
            "read-only" => self.read_only = value.as_bool()?,
            "size" => self.style.size_px = value.as_size()?,
            "tab-width" => self.tab_width = value.as_index()?.clamp(1, 16) as u8,
            _ => return Err(Mismatch::Unknown),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_of_counts_lines_and_the_last_column() {
        assert_eq!(end_of(Pos::new(2, 3), "ab"), Pos::new(2, 5));
        assert_eq!(end_of(Pos::new(2, 3), "a\nbc"), Pos::new(3, 2));
        assert_eq!(end_of(Pos::new(2, 3), "\n\n"), Pos::new(4, 0));
    }

    #[test]
    fn a_buffer_inserts_and_deletes_across_lines() {
        let mut b = TextBuffer::from_text("one\ntwo");
        b.insert(Pos::new(0, 3), " and\na half");
        assert_eq!(b.text(), "one and\na half\ntwo");
        b.delete(Pos::new(0, 3), Pos::new(2, 1));
        assert_eq!(b.text(), "onewo");
        assert_eq!(b.line_count(), Some(1));
        assert_eq!(b.line(0).as_deref(), Some("onewo"));
        assert_eq!(b.line(1), None);
    }

    #[test]
    fn undo_takes_back_a_run_of_typing_at_once() {
        let mut b = TextBuffer::new();
        for ch in ["h", "i", "\n", "t", "here"] {
            let at = end_of(Pos::ZERO, &b.text());
            b.insert(at, ch);
        }
        assert_eq!(b.text(), "hi\nthere");
        assert!(b.undo());
        assert_eq!(b.text(), "hi\n", "the second line was one run");
        assert!(b.undo());
        assert_eq!(b.text(), "hi");
        assert!(b.undo());
        assert_eq!(b.text(), "");
        assert!(!b.undo());
        assert!(b.redo());
        assert!(b.redo());
        assert_eq!(b.text(), "hi\n");
        b.insert(Pos::new(1, 0), "x");
        assert!(!b.redo(), "a new edit drops the redo history");
        b.delete(Pos::new(0, 1), Pos::new(1, 1));
        assert_eq!(b.text(), "h");
        assert!(b.undo());
        assert_eq!(b.text(), "hi\nx");
    }

    #[test]
    fn words_are_runs_of_one_class() {
        assert_eq!(word_at("let x_1 = f(a)", 5), (4, 7));
        assert_eq!(
            word_at("let x_1 = f(a)", 7),
            (4, 7),
            "at the end of a word is in it"
        );
        assert_eq!(
            word_at("let x_1 = f(a)", 11),
            (10, 11),
            "after `f` is still `f`"
        );
        assert_eq!(word_at("let x_1 = f(a)", 3), (0, 3));
        assert_eq!(word_at("a (b", 2), (1, 3), "a run of non-word characters");
        assert_eq!(word_at("", 0), (0, 0));
        assert_eq!(word_at("æøå bc", 0), (0, 6));
    }

    #[test]
    fn the_thumb_is_the_rows_share_of_the_lines_and_never_a_sliver() {
        // Ten rows of a hundred lines: a tenth of the track.
        assert_eq!(thumb_span(1000, 10, 100, 0, 20), (0, 100));
        // At the last top the thumb touches the bottom.
        assert_eq!(thumb_span(1000, 10, 100, 90, 20), (900, 100));
        // Halfway through the tops is halfway down the travel.
        assert_eq!(thumb_span(1000, 10, 100, 45, 20), (450, 100));
        // A million lines would make a sub-pixel thumb; the floor holds.
        assert_eq!(thumb_span(1000, 10, 1_000_000, 0, 20), (0, 20));
        // Everything fits: the thumb is the whole track and does not move.
        assert_eq!(thumb_span(1000, 50, 20, 0, 20), (0, 1000));
        assert_eq!(
            thumb_span(0, 10, 100, 5, 20),
            (0, 1),
            "a track with no height"
        );
    }

    #[test]
    fn boundaries_step_over_whole_characters() {
        let line = "aæb";
        assert_eq!(next_boundary(line, 1), 3);
        assert_eq!(prev_boundary(line, 3), 1);
        assert_eq!(prev_boundary(line, 0), 0);
        assert_eq!(next_boundary(line, 4), 4);
    }
}
