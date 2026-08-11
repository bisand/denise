//! A single-line editable text field.

use alloc::string::String;

use denise::{ElementState, InputEvent, KeyCode, Point, Radius, Rect, Role};
use denise_render::Canvas;
use denise_render::font::{self, BitmapFont};

use crate::widget::{Animation, Event, EventCtx, Handled, PaintCtx, VisualState, Widget};
use crate::widgets::style::{Align, draw_aligned, focus_ring, interactive_pair};

/// Half-period of the caret blink, in milliseconds.
const BLINK_MS: u64 = 500;

/// A single-line text field with a caret.
///
/// # What it does not do
///
/// No selection, no clipboard, no undo, no word motion. A kiosk field takes a
/// name, a PIN or a setpoint; the machinery those omissions would need is real
/// work that belongs with proper text handling in M4, and half of it is
/// meaningless without a font that can measure a substring.
///
/// # Blinking
///
/// The caret blinks only while the field has focus, and the tree only asks the
/// focused widget to animate. An unfocused panel therefore has nothing running on
/// a timer at all, which is the difference between a device that idles and one
/// that keeps a core awake for its whole service life. Typing resets the phase so
/// the caret stays solid while it is moving.
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
    scale: i32,
    radius: Radius,
    submit: Option<M>,
    password: bool,
    blink_epoch: u64,
    caret_on: bool,
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
            scale: 2,
            radius: Radius::Field,
            submit: None,
            password: false,
            blink_epoch: 0,
            caret_on: true,
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

    /// Sets the integer glyph scale.
    pub fn with_scale(mut self, scale: i32) -> Self {
        self.scale = scale.max(1);
        self
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
        font::ADVANCE * self.scale / 2
    }

    /// Characters that fit between the paddings.
    fn visible_chars(&self, bounds: Rect) -> usize {
        let inner = bounds.width - self.pad() * 2;
        ((inner / (font::ADVANCE * self.scale)).max(1)) as usize
    }

    /// First character to draw, given where the caret is.
    fn window_start(&self, bounds: Rect) -> usize {
        let visible = self.visible_chars(bounds);
        let mut first = self.first_visible.min(self.caret);
        if self.caret >= first + visible {
            first = self.caret + 1 - visible;
        }
        first
    }

    fn scroll_to_caret(&mut self, bounds: Rect) {
        self.first_visible = self.window_start(bounds);
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
        true
    }

    fn delete_before(&mut self) -> bool {
        if self.caret == 0 {
            return false;
        }
        let at = self.byte_of(self.caret - 1);
        self.text.remove(at);
        self.caret -= 1;
        true
    }

    fn delete_after(&mut self) -> bool {
        if self.caret >= self.len_chars() {
            return false;
        }
        let at = self.byte_of(self.caret);
        self.text.remove(at);
        true
    }
}

impl<M> Default for TextInput<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: Clone + 'static> Widget<M> for TextInput<M> {
    fn paint(&self, ctx: &PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        let radius = ctx.theme.radius(self.radius);
        let disabled = ctx.state.contains(VisualState::DISABLED);
        let (background, _) = interactive_pair(ctx.theme, Role::Base100, ctx.state);
        canvas.fill_rounded_rect(ctx.bounds, radius, background);
        canvas.stroke_rounded_rect(ctx.bounds, radius, 1, ctx.theme.color(Role::Base300));
        if ctx.state.contains(VisualState::FOCUSED) {
            focus_ring(ctx.theme, ctx.bounds, radius, canvas);
        }

        let pad = self.pad();
        let inner = Rect::from_edges(
            ctx.bounds.x + pad,
            ctx.bounds.y,
            ctx.bounds.right() - pad,
            ctx.bounds.bottom(),
        );
        let text_y = inner.y + Align::Center.offset(inner.height, font::CELL_HEIGHT * self.scale);

        if self.text.is_empty() && !self.placeholder.is_empty() {
            draw_aligned(
                canvas,
                inner,
                Align::Start,
                Align::Center,
                self.scale,
                &self.placeholder,
                ctx.theme
                    .color(Role::Base300)
                    .mix(ctx.theme.color(Role::BaseContent), 128),
            );
        } else {
            // Drawn glyph by glyph rather than by slicing the string, so scrolling
            // a long field costs no allocation in the paint path.
            let content = if disabled {
                ctx.theme.color(Role::Base300)
            } else {
                ctx.theme.color(Role::BaseContent)
            };
            let first = self.window_start(ctx.bounds);
            let visible = self.visible_chars(ctx.bounds);
            let mut pen = Point::new(inner.x, text_y);
            for ch in self.text.chars().skip(first).take(visible) {
                let ch = if self.password { '*' } else { ch };
                canvas.draw_glyph(font::BUILT_IN.glyph(ch), pen, self.scale, content);
                pen.x += font::ADVANCE * self.scale;
            }

            if ctx.state.contains(VisualState::FOCUSED) && self.caret_on && !disabled {
                let x = inner.x
                    + BitmapFont::caret_offset(&font::BUILT_IN, self.caret - first, self.scale);
                canvas.fill_rect(
                    Rect::new(x, text_y, self.scale, font::CELL_HEIGHT * self.scale),
                    ctx.theme.color(Role::Accent),
                );
            }
        }
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        match event {
            Event::FocusGained | Event::FocusLost => {
                self.wake_caret(ctx.now_ms);
                // Not `Handled`: nothing was consumed. The tree already repaints
                // on a focus change, so the caret appearing is covered.
                Handled::No
            }
            Event::Input(InputEvent::Text { ch }) if !ch.is_control() => {
                if self.insert(*ch) {
                    self.wake_caret(ctx.now_ms);
                    self.scroll_to_caret(ctx.bounds);
                    Handled::Yes
                } else {
                    Handled::No
                }
            }
            Event::Input(InputEvent::Key {
                code,
                state: ElementState::Down,
                ..
            }) => {
                let changed = match code {
                    KeyCode::Backspace => self.delete_before(),
                    KeyCode::Delete => self.delete_after(),
                    KeyCode::ArrowLeft => {
                        let moved = self.caret > 0;
                        self.caret = self.caret.saturating_sub(1);
                        moved
                    }
                    KeyCode::ArrowRight => {
                        let moved = self.caret < self.len_chars();
                        self.caret = (self.caret + 1).min(self.len_chars());
                        moved
                    }
                    KeyCode::Home => {
                        let moved = self.caret != 0;
                        self.caret = 0;
                        moved
                    }
                    KeyCode::End => {
                        let moved = self.caret != self.len_chars();
                        self.caret = self.len_chars();
                        moved
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
                };
                self.wake_caret(ctx.now_ms);
                self.scroll_to_caret(ctx.bounds);
                // Even a caret move that changed nothing must repaint, because the
                // caret itself is pixels.
                let _ = changed;
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
        let elapsed = now_ms.saturating_sub(self.blink_epoch);
        let on = (elapsed / BLINK_MS).is_multiple_of(2);
        let repaint = on != self.caret_on;
        self.caret_on = on;
        Animation {
            repaint,
            next_ms: Some(self.blink_epoch + (elapsed / BLINK_MS + 1) * BLINK_MS),
        }
    }
}
