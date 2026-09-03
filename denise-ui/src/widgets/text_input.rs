//! A single-line editable text field.

use alloc::string::String;

use denise::Pen;
use denise::{ElementState, InputEvent, KeyCode, Point, Radius, Rect, Role};
use denise_text::{TextEngine, TextStyle};

use crate::motion::Wake;
use crate::widget::{
    Animation, Event, EventCtx, Handled, MeasureCtx, Measured, Offer, PaintCtx, VisualState, Widget,
};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Payload, Property, PropertyKind, Value,
};
use crate::widgets::style::{Align, focus_ring, interactive_pair};

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
            Event::Input(InputEvent::Text { ch }) if !ch.is_control() => {
                if self.insert(*ch) {
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
                let bounds = ctx.bounds;
                self.scroll_to_caret(ctx.text, bounds);
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
