//! A single-line editable text field.

use alloc::string::String;

use denise::Pen;
use denise::{ElementState, InputEvent, KeyCode, Modifiers, Point, Radius, Rect, Role};
use denise_text::{TextEngine, TextStyle};

use crate::motion::Wake;
use crate::widget::{
    Animation, Event, EventCtx, Handled, MeasureCtx, Measured, Offer, PaintCtx, VisualState, Widget,
};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Payload, Property, PropertyKind, Value,
};
use crate::widgets::style::{Align, DOUBLE_CLICK_MS, focus_ring, interactive_pair};
use crate::widgets::text_area::is_word;

/// Half-period of the caret blink, in milliseconds.
const BLINK_MS: u64 = 500;

/// A single-line text field with a caret.
///
/// # Selecting
///
/// Taking focus selects everything, which is what makes Tab-and-type replace a
/// setpoint rather than append to it. A press places the caret, a second on the
/// same spot takes the word under it, a third takes the whole field, and
/// dragging — with a mouse or a finger — extends from where the press landed.
/// Shift extends with the arrows, Home and End; ⌘A or Ctrl+A takes everything.
/// Typing, Backspace and Delete replace what is selected.
///
/// # What it still does not do
///
/// No clipboard, no undo, no word motion from the keyboard. A kiosk field takes
/// a name, a PIN or a setpoint, and those three are the ones a panel with no
/// physical keyboard cannot ask for anyway; [`TextArea`](super::TextArea) is
/// where an editor's machinery lives.
///
/// # Blinking
///
/// The caret blinks only while the field has focus: taking focus requests
/// animation, and losing it makes [`Widget::animate`] answer `None`, which is
/// how a widget hands the CPU back. An unfocused panel therefore has nothing
/// running on a timer at all — the difference between a device that idles and
/// one that keeps a core awake for its whole service life. Typing resets the
/// phase so the caret stays solid while it is moving.
///
/// A blink damages the whole field rather than the caret, because
/// [`Widget::animate`] reports *that* something changed, not *where*. On a Pi 3
/// that is 26 kpx twice a second — 58 µs, or 0.35% of one 60 Hz frame — against
/// the 32 px the caret actually occupies. The 800× coarseness is real and the
/// cost of removing it is a wider trait; the measurement is why it has not been
/// paid.
#[derive(Clone, Debug)]
pub struct TextInput<M> {
    text: String,
    placeholder: String,
    /// Caret position as a **character** index, not a byte offset.
    caret: usize,
    /// First character drawn, for fields wider than their box.
    first_visible: usize,
    max_chars: usize,
    style: TextStyle,
    radius: Radius,
    submit: Option<M>,
    password: bool,
    blink_epoch: u64,
    caret_on: bool,
    /// Whether the field currently has focus, mirrored from the focus events.
    /// `animate` has no context to ask the tree, and this is what lets it stop
    /// asking for frames the moment focus moves away.
    has_focus: bool,
    /// The other end of the selection, as a character index, or `None` when
    /// there is only a caret: `anchor..caret` in whichever order they fall.
    anchor: Option<usize>,
    /// A press is still down in the field, so moving extends the selection.
    dragging: bool,
    /// Where the last press landed, when, and how many have stacked up on that
    /// spot: one places the caret, two take the word, three take everything.
    clicks: Option<(usize, u64, u8)>,
}

impl<M> TextInput<M> {
    /// An empty field.
    pub fn new() -> Self {
        Self {
            text: String::new(),
            placeholder: String::new(),
            caret: 0,
            first_visible: 0,
            max_chars: 256,
            style: TextStyle::built_in(16),
            radius: Radius::Field,
            submit: None,
            password: false,
            blink_epoch: 0,
            caret_on: true,
            has_focus: false,
            anchor: None,
            dragging: false,
            clicks: None,
        }
    }

    /// Sets the text shown when the field is empty.
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Sets the message emitted when Enter is pressed.
    pub fn with_submit(mut self, message: M) -> Self {
        self.submit = Some(message);
        self
    }

    /// Caps the number of characters the field will hold.
    pub fn with_max_chars(mut self, max: usize) -> Self {
        self.max_chars = max;
        self
    }

    /// Sets the font and size.
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets the size, keeping the font.
    pub fn with_size(mut self, size_px: u16) -> Self {
        self.style.size_px = size_px;
        self
    }

    /// The font and size this field draws in.
    #[inline]
    pub const fn style(&self) -> TextStyle {
        self.style
    }

    /// Draws every character as `*`. The text is still stored in the clear —
    /// this hides a PIN from someone standing behind the panel, and nothing more.
    pub fn with_password(mut self, password: bool) -> Self {
        self.password = password;
        self
    }

    /// The current contents.
    #[inline]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Replaces the contents, putting the caret at the end.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.caret = self.len_chars();
        self.first_visible = 0;
        // The anchor indexed text that is no longer there.
        self.anchor = None;
    }

    /// Replaces the font and size.
    pub fn set_style(&mut self, style: TextStyle) {
        self.style = style;
    }

    /// Empties the field.
    pub fn clear(&mut self) {
        self.set_text(String::new());
    }

    /// Caret position, as a character index.
    #[inline]
    pub const fn caret(&self) -> usize {
        self.caret
    }

    /// The selection as character indices, low end first, or `None` when there
    /// is only a caret.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor.filter(|a| *a != self.caret)?;
        Some((anchor.min(self.caret), anchor.max(self.caret)))
    }

    /// The selected text, or `None` when nothing is selected.
    pub fn selected_text(&self) -> Option<&str> {
        let (from, to) = self.selection()?;
        Some(&self.text[self.byte_of(from)..self.byte_of(to)])
    }

    /// Selects everything, with the caret at the end.
    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.caret = self.len_chars();
    }

    /// Selects `from..to`, clamped to the text, with the caret at `to`.
    ///
    /// The pair may be given either way round: the caret lands on `to`, which is
    /// the end a further Shift-arrow moves.
    pub fn select_range(&mut self, from: usize, to: usize) {
        let len = self.len_chars();
        self.anchor = Some(from.min(len));
        self.caret = to.min(len);
    }

    /// Drops the selection, leaving the caret where it is.
    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }

    #[inline]
    fn len_chars(&self) -> usize {
        self.text.chars().count()
    }

    /// Byte offset of character `index`, or the end of the string.
    fn byte_of(&self, index: usize) -> usize {
        self.text
            .char_indices()
            .nth(index)
            .map_or(self.text.len(), |(offset, _)| offset)
    }

    /// Horizontal padding inside the field's bounds.
    #[inline]
    const fn pad(&self) -> i32 {
        self.style.size_px as i32 / 3
    }

    /// The field's inner rectangle, inside the padding.
    fn inner(&self, bounds: Rect) -> Rect {
        Rect::from_edges(
            bounds.x + self.pad(),
            bounds.y,
            bounds.right() - self.pad(),
            bounds.bottom(),
        )
    }

    /// Width of characters `from..to` as they are displayed.
    ///
    /// Measured rather than counted. With a proportional font a caret placed by
    /// multiplying an index by an advance is wrong everywhere except after the
    /// first character, and wrong in a way that looks like a rendering glitch
    /// rather than an arithmetic mistake.
    fn run_width(&self, engine: &mut TextEngine, from: usize, to: usize) -> i32 {
        if from >= to {
            return 0;
        }
        if self.password {
            return engine.measure_line(self.style, "*") * (to - from) as i32;
        }
        let (start, end) = (self.byte_of(from), self.byte_of(to));
        engine.measure_line(self.style, &self.text[start..end])
    }

    /// First character to draw, given where the caret is and how wide the box is.
    fn window_start(&self, engine: &mut TextEngine, bounds: Rect) -> usize {
        let available = self.inner(bounds).width;
        let mut first = self.first_visible.min(self.caret);
        // Walks rather than bisects: a kiosk field holds a name or a setpoint, and
        // the loop runs once per character that scrolled off since last frame,
        // which is almost always one.
        while first < self.caret && self.run_width(engine, first, self.caret) > available {
            first += 1;
        }
        first
    }

    /// Horizontal offset of the caret from the field's left edge.
    ///
    /// Measured through the engine rather than counted as characters times a
    /// width, which is the only thing that works with a proportional font.
    pub fn caret_x(&self, engine: &mut TextEngine, bounds: Rect) -> i32 {
        let first = self.window_start(engine, bounds);
        self.pad() + self.run_width(engine, first, self.caret)
    }

    fn scroll_to_caret(&mut self, engine: &mut TextEngine, bounds: Rect) {
        self.first_visible = self.window_start(engine, bounds);
    }

    /// Restarts the blink so the caret is solid while it is being moved.
    fn wake_caret(&mut self, now_ms: u64) {
        self.blink_epoch = now_ms;
        self.caret_on = true;
    }

    fn insert(&mut self, ch: char) -> bool {
        if self.len_chars() >= self.max_chars {
            return false;
        }
        let at = self.byte_of(self.caret);
        self.text.insert(at, ch);
        self.caret += 1;
        // An edit collapses the selection. Leaving the anchor where it was made
        // the character just typed look selected, so the next keystroke
        // replaced it — a field that kept only its last letter.
        self.anchor = None;
        true
    }

    fn delete_before(&mut self) -> bool {
        if self.caret == 0 {
            return false;
        }
        let at = self.byte_of(self.caret - 1);
        self.text.remove(at);
        self.caret -= 1;
        self.anchor = None;
        true
    }

    fn delete_after(&mut self) -> bool {
        if self.caret >= self.len_chars() {
            return false;
        }
        let at = self.byte_of(self.caret);
        self.text.remove(at);
        self.anchor = None;
        true
    }

    /// Removes the selection, reporting whether there was one. The caret lands
    /// where the selection started, which is where typing continues from.
    fn delete_selection(&mut self) -> bool {
        let Some((from, to)) = self.selection() else {
            return false;
        };
        let (start, end) = (self.byte_of(from), self.byte_of(to));
        self.text.replace_range(start..end, "");
        self.caret = from;
        self.anchor = None;
        true
    }

    /// Moves the caret, extending the selection or dropping it.
    fn move_to(&mut self, to: usize, extend: bool) {
        if extend {
            self.anchor.get_or_insert(self.caret);
        } else {
            self.anchor = None;
        }
        self.caret = to.min(self.len_chars());
    }

    /// The character index nearest `x`, which is where a press puts the caret.
    ///
    /// Nearest, not "the character containing it": a press in the right half of
    /// a letter belongs after it. Measured through the engine one character at a
    /// time, the same way [`caret_x`](Self::caret_x) measures, so the caret
    /// lands exactly where the press was drawn to be.
    fn index_at(&self, engine: &mut TextEngine, bounds: Rect, x: i32) -> usize {
        let inner = self.inner(bounds);
        let first = self.window_start(engine, bounds);
        let target = x - inner.x;
        if target <= 0 {
            return first;
        }
        let len = self.len_chars();
        let mut index = first;
        let mut before = 0;
        while index < len {
            let after = self.run_width(engine, first, index + 1);
            if target < (before + after) / 2 {
                break;
            }
            before = after;
            index += 1;
        }
        index
    }

    /// The run of word characters — or of non-word ones — around `index`.
    ///
    /// The word before wins at its end, so a double-click just past the last
    /// letter takes the word rather than the space after it. Same rule as
    /// [`TextArea`](super::TextArea), because it is the same gesture.
    fn word_bounds(&self, index: usize) -> (usize, usize) {
        let len = self.len_chars();
        let index = index.min(len);
        let at = |i: usize| self.text.chars().nth(i);
        let class = match (index.checked_sub(1).and_then(&at), at(index)) {
            (Some(b), _) if is_word(b) => true,
            (_, Some(a)) => is_word(a),
            (Some(b), None) => is_word(b),
            (None, None) => return (index, index),
        };
        let mut start = index;
        while start > 0 && at(start - 1).is_some_and(|c| is_word(c) == class) {
            start -= 1;
        }
        let mut end = index;
        while end < len && at(end).is_some_and(|c| is_word(c) == class) {
            end += 1;
        }
        (start, end)
    }

    /// What a press at `index` means, given what came before it.
    ///
    /// One places the caret, two takes the word, three takes everything, and a
    /// fourth starts the count again — so holding a finger down and tapping
    /// cycles rather than sticking on "everything". The spot has to match: a
    /// second press a few characters away is a new first press, not a double.
    fn click_count(&mut self, index: usize, now_ms: u64) -> u8 {
        let count = match self.clicks {
            Some((at, when, count))
                if at == index && now_ms.saturating_sub(when) <= DOUBLE_CLICK_MS =>
            {
                count % 3 + 1
            }
            _ => 1,
        };
        self.clicks = Some((index, now_ms, count));
        count
    }

    /// A press or a tap: places the caret, takes a word, or takes everything.
    fn press(&mut self, position: Point, extend: bool, ctx: &mut EventCtx<'_, M>) -> Handled {
        let bounds = ctx.bounds;
        let index = self.index_at(ctx.text, bounds, position.x);
        match self.click_count(index, ctx.now_ms) {
            2 => {
                let (start, end) = self.word_bounds(index);
                self.anchor = Some(start);
                self.caret = end;
                self.dragging = false;
            }
            3 => {
                self.select_all();
                self.dragging = false;
            }
            _ => {
                self.move_to(index, extend);
                self.dragging = true;
            }
        }
        self.wake_caret(ctx.now_ms);
        self.scroll_to_caret(ctx.text, bounds);
        Handled::Yes
    }

    /// The pointer moved with the press still down: the selection follows it.
    fn drag(&mut self, position: Point, ctx: &mut EventCtx<'_, M>) -> Handled {
        let bounds = ctx.bounds;
        let index = self.index_at(ctx.text, bounds, position.x);
        if index == self.caret {
            return Handled::No;
        }
        self.move_to(index, true);
        self.wake_caret(ctx.now_ms);
        self.scroll_to_caret(ctx.text, bounds);
        Handled::Yes
    }
}

impl<M> Default for TextInput<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: Clone + 'static> Widget<M> for TextInput<M> {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn measure(&self, ctx: &mut MeasureCtx<'_>, _offered: Offer) -> Measured {
        // A field is as wide as you make it — that is what a field is — but its
        // height is one line of its own text in a field-sized box.
        let line = ctx.text.line_height(self.style);
        Measured::tall(line.max(ctx.theme.metrics.size_field).max(1))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        let radius = ctx.theme.radius(self.radius);
        let disabled = ctx.state.contains(VisualState::DISABLED);
        let focused = ctx.state.contains(VisualState::FOCUSED);
        let (background, _) = interactive_pair(ctx.theme, Role::Base100, ctx.state);
        canvas.fill_rounded_rect(ctx.bounds, radius, background);
        canvas.stroke_rounded_rect(ctx.bounds, radius, 1, ctx.theme.color(Role::Base300));
        if focused {
            focus_ring(ctx.theme, ctx.bounds, radius, canvas);
        }

        let inner = self.inner(ctx.bounds);
        let line_height = ctx.text.line_height(self.style);
        let top = inner.y + Align::Center.offset(inner.height, line_height);
        // Text is clipped to the inner box, so a value longer than the field
        // scrolls under the border rather than over it.
        let mut clipped = canvas.with_clip(inner);

        // Under the text, and only while the field has focus: a highlight on a
        // field nobody is typing into reads as a field that still has the
        // keyboard. The selection itself is kept, so focus coming back shows it.
        if focused
            && !disabled
            && let Some((from, to)) = self.selection()
        {
            let first = self.window_start(ctx.text, ctx.bounds);
            let start = from.max(first);
            if to > start {
                let x0 = inner.x + self.run_width(ctx.text, first, start);
                let x1 = inner.x + self.run_width(ctx.text, first, to);
                clipped.fill_rect(
                    Rect::new(x0, top, x1 - x0, line_height),
                    ctx.theme.color(Role::Accent).with_alpha(60),
                );
            }
        }

        if self.text.is_empty() {
            if !self.placeholder.is_empty() {
                let hint = ctx
                    .theme
                    .color(Role::Base300)
                    .mix(ctx.theme.color(Role::BaseContent), 128);
                ctx.text.draw(
                    &mut clipped,
                    self.style,
                    Point::new(inner.x, top),
                    &self.placeholder,
                    hint,
                );
            }
        } else {
            let content = if disabled {
                ctx.theme.color(Role::Base300)
            } else {
                ctx.theme.color(Role::BaseContent)
            };
            let first = self.window_start(ctx.text, ctx.bounds);
            if self.password {
                // Drawn one at a time rather than by building a string of stars,
                // because a paint path that allocates is a paint path that can
                // fail on a device with no memory left.
                let advance = ctx.text.measure_line(self.style, "*");
                let count = self.len_chars().saturating_sub(first);
                for i in 0..count {
                    let x = inner.x + advance * i as i32;
                    if x > inner.right() {
                        break;
                    }
                    ctx.text
                        .draw(&mut clipped, self.style, Point::new(x, top), "*", content);
                }
            } else {
                let start = self.byte_of(first);
                ctx.text.draw(
                    &mut clipped,
                    self.style,
                    Point::new(inner.x, top),
                    &self.text[start..],
                    content,
                );
            }
        }

        if focused && self.caret_on && !disabled {
            let first = self.window_start(ctx.text, ctx.bounds);
            let x = inner.x + self.run_width(ctx.text, first, self.caret);
            let width = (i32::from(self.style.size_px) / 10).max(1);
            clipped.fill_rect(
                Rect::new(x, top, width, line_height),
                ctx.theme.color(Role::Accent),
            );
        }
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        match event {
            Event::FocusGained => {
                self.has_focus = true;
                // Tabbing into a field offers its contents for replacement,
                // which is what makes Tab-and-type work on a setpoint. A press
                // is delivered *after* the focus it caused, so clicking into a
                // field still collapses this to a caret where the finger landed.
                self.select_all();
                self.wake_caret(ctx.now_ms);
                ctx.request_animation();
                // Not `Handled`: nothing was consumed. The tree already repaints
                // on a focus change, so the caret appearing is covered.
                Handled::No
            }
            Event::FocusLost => {
                self.has_focus = false;
                self.wake_caret(ctx.now_ms);
                Handled::No
            }
            // A finger is a pointer here: the same three counts, because a
            // panel with an on-screen keyboard has no other way to take a word.
            Event::Input(InputEvent::PointerButton {
                state: ElementState::Down,
                position,
                modifiers,
                ..
            }) => self.press(*position, modifiers.contains(Modifiers::SHIFT), ctx),
            Event::Input(InputEvent::TouchDown { position, .. }) => {
                self.press(*position, false, ctx)
            }
            Event::Input(InputEvent::PointerMoved { position })
            | Event::Input(InputEvent::TouchMoved { position, .. })
                if self.dragging =>
            {
                self.drag(*position, ctx)
            }
            // The release ends the drag, and so does the tree taking the press
            // away — a scene pushed over the field, or its node hidden. That
            // arrives as `PressCancelled` and as no pointer event at all, and a
            // drag left armed would follow the next hover across the field.
            Event::Input(InputEvent::PointerButton {
                state: ElementState::Up,
                ..
            })
            | Event::Input(InputEvent::TouchUp { .. })
            | Event::PressCancelled => {
                self.dragging = false;
                Handled::No
            }
            Event::Input(InputEvent::Text { ch }) if !ch.is_control() => {
                // What is selected is what typing replaces.
                let replaced = self.delete_selection();
                let inserted = self.insert(*ch);
                if replaced || inserted {
                    self.wake_caret(ctx.now_ms);
                    let bounds = ctx.bounds;
                    self.scroll_to_caret(ctx.text, bounds);
                    Handled::Yes
                } else {
                    Handled::No
                }
            }
            Event::Input(InputEvent::Key {
                code,
                state: ElementState::Down,
                modifiers,
                ..
            }) => {
                let extend = modifiers.contains(Modifiers::SHIFT);
                // Ctrl on a keyboard, Command on a Mac: one of the two is
                // "select all" on every machine this runs on.
                let shortcut =
                    modifiers.contains(Modifiers::CTRL) || modifiers.contains(Modifiers::SUPER);
                match code {
                    KeyCode::A if shortcut => self.select_all(),
                    // A selection is what Backspace and Delete take first; only
                    // an empty one falls through to the character either side.
                    KeyCode::Backspace => {
                        if !self.delete_selection() {
                            self.delete_before();
                        }
                    }
                    KeyCode::Delete => {
                        if !self.delete_selection() {
                            self.delete_after();
                        }
                    }
                    // An arrow with a selection and no Shift collapses to that
                    // end rather than moving from the caret, which is what puts
                    // the caret back where a person is looking.
                    KeyCode::ArrowLeft => match self.selection() {
                        Some((from, _)) if !extend => {
                            self.caret = from;
                            self.anchor = None;
                        }
                        _ => {
                            let to = self.caret.saturating_sub(1);
                            self.move_to(to, extend);
                        }
                    },
                    KeyCode::ArrowRight => match self.selection() {
                        Some((_, to)) if !extend => {
                            self.caret = to;
                            self.anchor = None;
                        }
                        _ => {
                            let to = self.caret.saturating_add(1);
                            self.move_to(to, extend);
                        }
                    },
                    KeyCode::Home => self.move_to(0, extend),
                    KeyCode::End => {
                        let end = self.len_chars();
                        self.move_to(end, extend);
                    }
                    KeyCode::Enter | KeyCode::NumpadEnter => {
                        if let Some(message) = self.submit.clone() {
                            ctx.emit(message);
                        }
                        // Consumed either way: Enter in a field must not fall
                        // through and activate something else.
                        return Handled::Yes;
                    }
                    _ => return Handled::No,
                }
                self.wake_caret(ctx.now_ms);
                let bounds = ctx.bounds;
                self.scroll_to_caret(ctx.text, bounds);
                // Even a caret move that changed nothing must repaint, because the
                // caret itself is pixels.
                Handled::Yes
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
            // Blinking is for the field being typed into. Answering `None` is
            // what takes this widget out of the animating set — the caret is
            // not drawn without focus, so there is nothing left to repaint.
            return Animation::NONE;
        }
        let elapsed = now_ms.saturating_sub(self.blink_epoch);
        let on = (elapsed / BLINK_MS).is_multiple_of(2);
        let repaint = on != self.caret_on;
        self.caret_on = on;
        Animation {
            repaint,
            // A deadline, not a frame rate: the caret flips at the end of each
            // blink and wants exactly one wake to do it. Halving the tree's
            // animation rate must not halve the blink, and turning motion off
            // must not stop it — a caret that has stopped blinking is a field
            // that looks like it has lost focus.
            //
            // Saturating, because `now_ms` is the application's clock and this
            // widget does not get to assume anything about it. A host that
            // counts from the Unix epoch, or a fuzzer that passes `u64::MAX`,
            // must not be able to panic a panel through the caret blink.
            next: Wake::At(
                self.blink_epoch.saturating_add(
                    (elapsed / BLINK_MS)
                        .saturating_add(1)
                        .saturating_mul(BLINK_MS),
                ),
            ),
        }
    }

    /// Blinking is a schedule, so it survives [`Motion::None`](crate::Motion)
    /// unchanged — there is nothing to land, and stopping it would be a
    /// regression dressed up as a preference.
    fn snap(&mut self, now_ms: u64) -> Animation {
        Widget::<M>::animate(self, now_ms)
    }
}

impl<M> Describe for TextInput<M> {
    const KIND: &'static str = "text-input";
    const DOC: &'static str = "A line of text somebody types into.";
    const GROUP: Group = Group::Input;
    const ICON: &'static denise::icon::Icon = &super::icons::TEXT_INPUT;

    const PROPERTIES: &'static [Property] = &[
        Property::new("text", PropertyKind::Text, "Initial contents."),
        Property::new(
            "placeholder",
            PropertyKind::Text,
            "Shown while the field is empty.",
        ),
        Property::new(
            "on-submit",
            PropertyKind::Message(Payload::None),
            "The message emitted on Enter.",
        ),
        Property::new(
            "max-chars",
            PropertyKind::Int { min: 1, max: 4096 },
            "How many characters the field will hold.",
        ),
        Property::new(
            "password",
            PropertyKind::Bool,
            "Draw every character as `*`. The text is still stored in the clear.",
        ),
        Property::new(
            "size",
            PropertyKind::Int { min: 6, max: 96 },
            "Text size in logical pixels.",
        )
        .in_pixels(),
    ];

    fn get(&self, name: &str) -> Option<Value> {
        Some(match name {
            "text" => Value::text(self.text.as_str()),
            "placeholder" => Value::text(self.placeholder.as_str()),
            // The message is the application's, and this crate has never seen
            // its type. See the `describe` module docs.
            "on-submit" => return None,
            "max-chars" => Value::Int(i32::try_from(self.max_chars).unwrap_or(i32::MAX)),
            "password" => Value::Bool(self.password),
            "size" => Value::Int(i32::from(self.style.size_px)),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            // Through the setter, which puts the caret at the end and resets the
            // window: assigning the field would leave a caret pointing into text
            // that is no longer there.
            "text" => self.set_text(value.as_text()?),
            "placeholder" => self.placeholder = value.as_text()?,
            "on-submit" => return Err(Mismatch::Supplied),
            "max-chars" => self.max_chars = value.as_index()?,
            "password" => self.password = value.as_bool()?,
            "size" => self.style.size_px = value.as_size()?,
            _ => return Err(Mismatch::Unknown),
        }
        Ok(())
    }
}
