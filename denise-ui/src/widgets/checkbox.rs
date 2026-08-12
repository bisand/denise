//! A box, a tick, and a boolean.

use alloc::string::String;

use denise::{ElementState, InputEvent, KeyCode, Point, Radius, Rect, Role, Theme};
use denise_render::Canvas;
use denise_text::{TextEngine, TextStyle};

use crate::widget::{Event, EventCtx, Handled, PaintCtx, VisualState, Widget};
use crate::widgets::style::{Align, draw_aligned, focus_ring, interactive_pair};

/// A checkbox with an optional label beside it.
///
/// The message is a function of the **new** value rather than a fixed value, so
/// an application matches on what the checkbox became rather than looking the
/// widget up afterwards. An enum's tuple variant already is such a function:
///
/// ```ignore
/// enum Message { Muted(bool) }
/// Checkbox::new("Mute", Message::Muted)
/// ```
///
/// A plain `fn` pointer rather than a closure, so this needs no allocation, no
/// `M: Clone`, and works in `no_std`.
///
/// # Why there is no indeterminate state
///
/// The third state only means anything for a checkbox that summarises other
/// checkboxes — a parent over a list of children — and neither the list nor the
/// hierarchy exists here yet. It is also additive when it does: HTML keeps
/// `indeterminate` as a property separate from `checked` precisely because it is
/// a way of *drawing* a checkbox rather than a third value it can hold. So this
/// stays a `bool` and gains a flag later, rather than becoming an enum everybody
/// has to match on for a case nothing can currently produce.
#[derive(Clone, Debug)]
pub struct Checkbox<M> {
    label: String,
    checked: bool,
    message: Option<fn(bool) -> M>,
    role: Role,
    style: TextStyle,
}

impl<M> Checkbox<M> {
    /// An unchecked box whose message is built from the value it changes to.
    pub fn new(label: impl Into<String>, message: fn(bool) -> M) -> Self {
        Self {
            label: label.into(),
            checked: false,
            message: Some(message),
            role: Role::Primary,
            style: TextStyle::built_in(16),
        }
    }

    /// A checkbox that emits nothing, for a value the application reads rather
    /// than reacts to.
    pub fn inert(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            checked: false,
            message: None,
            role: Role::Primary,
            style: TextStyle::built_in(16),
        }
    }

    /// Sets the initial value.
    pub fn with_checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// Sets the colour role of the filled box. The tick comes from the theme's
    /// pairing, so it stays readable whichever role and theme are chosen.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Sets the label's font and size.
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets the label's size, keeping the font.
    pub fn with_size(mut self, size_px: u16) -> Self {
        self.style.size_px = size_px;
        self
    }

    /// Whether the box is ticked.
    #[inline]
    pub const fn checked(&self) -> bool {
        self.checked
    }

    /// Sets the value **without emitting anything**.
    ///
    /// The message reports what a person did. An application that assigns here
    /// and then receives its own message back would either loop or have to guard
    /// against itself, which is the bug this rule exists to prevent.
    pub fn set_checked(&mut self, checked: bool) {
        self.checked = checked;
    }

    /// The current label.
    #[inline]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Replaces the label.
    pub fn set_label(&mut self, label: impl Into<String>) {
        self.label = label.into();
    }

    /// Replaces the colour role.
    pub fn set_role(&mut self, role: Role) {
        self.role = role;
    }

    /// Replaces the label's font and size.
    ///
    /// For an application that registers a font after building its tree, which is
    /// the ordinary case: the tree has to exist before anyone knows whether the
    /// font file was there.
    pub fn set_style(&mut self, style: TextStyle) {
        self.style = style;
    }

    /// Width this checkbox needs for its box, its gap and its label.
    ///
    /// Takes the theme because the box is a theme metric — a touch theme's box is
    /// 28 logical pixels where a mouse theme's is 20 — and the engine because
    /// with a proportional font the label's width is not the character count
    /// times anything.
    pub fn preferred_width(&self, theme: &Theme, engine: &mut TextEngine) -> i32 {
        let side = theme.metrics.size_selector;
        let text = engine.measure_line(self.style, &self.label);
        if self.label.is_empty() {
            side
        } else {
            side + gap(side) + text
        }
    }
}

/// Space between the box and its label.
#[inline]
const fn gap(side: i32) -> i32 {
    // `Ord::max` is not const yet, and this is const so `preferred_width` and the
    // paint path cannot drift apart.
    if side < 2 { 1 } else { side / 2 }
}

/// The box itself: a square at the leading edge, centred vertically.
///
/// Clamped to the height it is given, so a checkbox in a row shorter than the
/// theme's selector size draws a smaller box rather than one that overflows into
/// its neighbours.
fn box_rect(bounds: Rect, theme: &Theme) -> Rect {
    let side = theme
        .metrics
        .size_selector
        .min(bounds.height)
        .min(bounds.width)
        .max(1);
    Rect::new(bounds.x, bounds.y + (bounds.height - side) / 2, side, side)
}

/// How heavy the tick's stroke is, for a box `side` pixels across.
///
/// Derived from the box rather than from `Metrics::border`: the border metric is
/// about the line around a control, and tying a checkmark's weight to it means a
/// theme that wants hairline borders gets an illegible tick.
#[inline]
const fn tick_weight(side: i32) -> i32 {
    if side / 8 < 2 { 2 } else { side / 8 }
}

/// Draws the tick inside `area`.
///
/// Two segments, at the proportions a checkmark is normally drawn at. Thickness
/// is faked by drawing the pair several times offset downwards, because
/// [`Canvas::draw_line`] is deliberately one pixel wide and there is no
/// thick-line primitive — for a stroke at roughly 45° a vertical offset gives an
/// effective width of about `t / √2`, which is close enough that nobody counting
/// pixels on a 20-pixel box would notice, and far better than the anaemic hairline
/// a single pass gives.
fn draw_tick(canvas: &mut Canvas<'_>, area: Rect, color: denise::Color, thickness: i32) {
    let s = area.width;
    // Proportions of the box, in eighths and sixteenths so the arithmetic stays
    // integral at every size a selector metric can produce.
    let start = Point::new(area.x + s * 7 / 32, area.y + s * 17 / 32);
    let elbow = Point::new(area.x + s * 13 / 32, area.y + s * 23 / 32);
    let end = Point::new(area.x + s * 25 / 32, area.y + s * 9 / 32);

    for step in 0..thickness.max(1) {
        let dy = step;
        canvas.draw_line(
            Point::new(start.x, start.y + dy),
            Point::new(elbow.x, elbow.y + dy),
            color,
        );
        canvas.draw_line(
            Point::new(elbow.x, elbow.y + dy),
            Point::new(end.x, end.y + dy),
            color,
        );
    }
}

impl<M: 'static> Widget<M> for Checkbox<M> {
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        let area = box_rect(ctx.bounds, ctx.theme);
        // Never more than a half-side, or the "rounded rect" is a circle with the
        // corners guessed at — which is what a radio button should look like and a
        // checkbox should not.
        let radius = ctx.theme.radius(Radius::Selector).min(area.width / 2);

        // The label always uses the base pairing, so it stays plain text on the
        // panel rather than taking the box's role colour. Reading it through
        // `interactive_pair` is what makes it mute itself when disabled.
        let (surface, on_surface) = interactive_pair(ctx.theme, Role::Base100, ctx.state);

        if self.checked {
            let (fill, mark) = interactive_pair(ctx.theme, self.role, ctx.state);
            canvas.fill_rounded_rect(area, radius, fill);
            draw_tick(canvas, area, mark, tick_weight(area.width));
        } else {
            canvas.fill_rounded_rect(area, radius, surface);
            canvas.stroke_rounded_rect(
                area,
                radius,
                ctx.theme.metrics.border,
                ctx.theme.color(Role::Base300),
            );
        }

        if ctx.state.contains(VisualState::FOCUSED) {
            // Around the whole widget, label included, because the label is part
            // of the hit area and a ring around only the box would say otherwise.
            focus_ring(
                ctx.theme,
                ctx.bounds,
                ctx.theme.radius(Radius::Field),
                canvas,
            );
        }

        if self.label.is_empty() {
            return;
        }
        let text = Rect::from_edges(
            area.right() + gap(area.width),
            ctx.bounds.y,
            ctx.bounds.right(),
            ctx.bounds.bottom(),
        );
        if !text.is_empty() {
            draw_aligned(
                canvas,
                ctx.text,
                self.style,
                text,
                (Align::Start, Align::Center),
                &self.label,
                on_surface,
            );
        }
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        let toggled = match event {
            Event::Input(InputEvent::PointerButton {
                state: ElementState::Up,
                position,
                ..
            }) => ctx.bounds.contains(*position),
            Event::Input(InputEvent::TouchUp {
                position,
                cancelled: false,
                ..
            }) => ctx.bounds.contains(*position),
            // Space and not Enter. Enter belongs to the form's default action, and
            // a checkbox that swallows it is why a dialog stops submitting when
            // focus happens to be sitting on one.
            Event::Input(InputEvent::Key {
                code: KeyCode::Space,
                state: ElementState::Down,
                repeat: false,
                ..
            }) => ctx.state.contains(VisualState::FOCUSED),
            _ => return Handled::No,
        };
        if !toggled {
            return Handled::No;
        }
        self.checked = !self.checked;
        if let Some(message) = self.message {
            ctx.emit(message(self.checked));
        }
        Handled::Yes
    }

    fn accepts_pointer(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use denise::theme;

    /// The box tracks the theme's selector metric, which a touch theme makes
    /// larger — the whole reason that metric exists.
    #[test]
    fn the_box_follows_the_theme_and_sits_at_the_leading_edge() {
        let bounds = Rect::new(10, 20, 200, 40);

        let mouse = box_rect(bounds, &theme::DARK);
        assert_eq!(mouse.width, theme::DARK.metrics.size_selector);
        assert_eq!(mouse.x, bounds.x, "the box is at the leading edge");
        assert_eq!(
            mouse.y + mouse.height / 2,
            bounds.y + bounds.height / 2,
            "and centred against the label beside it"
        );

        let touch = theme::DARK.with_metrics(denise::theme::Metrics::TOUCH);
        assert!(box_rect(bounds, &touch).width > mouse.width);
    }

    /// A checkbox in a row shorter than the metric draws a smaller box rather
    /// than one that overflows into whatever is above and below it.
    #[test]
    fn a_short_row_shrinks_the_box_instead_of_overflowing() {
        let bounds = Rect::new(0, 0, 200, 12);
        let area = box_rect(bounds, &theme::DARK);
        assert!(area.width <= 12);
        assert!(area.height <= bounds.height);
        assert!(area.width >= 1, "and never collapses to nothing");
    }

    /// Degenerate bounds must still produce a drawable square. A zero-width box
    /// reaches `fill_rounded_rect` with a radius of zero and a canvas that has to
    /// cope; a *negative* one would be a rectangle with inverted edges.
    #[test]
    fn degenerate_bounds_still_give_a_square_with_area() {
        for bounds in [
            Rect::new(0, 0, 0, 0),
            Rect::new(0, 0, 1, 40),
            Rect::new(0, 0, 40, 1),
        ] {
            let area = box_rect(bounds, &theme::DARK);
            assert!(area.width >= 1 && area.height >= 1, "{bounds:?}");
            assert_eq!(area.width, area.height, "{bounds:?} is not square");
        }
    }

    /// The label is measured, not guessed, and a checkbox with no label is just
    /// the box — no trailing gap for text that is not there.
    #[test]
    fn the_preferred_width_covers_the_box_the_gap_and_the_label() {
        let mut engine = TextEngine::new();
        let style = TextStyle::built_in(16);
        let side = theme::DARK.metrics.size_selector;

        let labelled: Checkbox<()> = Checkbox::inert("Enable logging");
        let text = engine.measure_line(style, "Enable logging");
        assert_eq!(
            labelled.preferred_width(&theme::DARK, &mut engine),
            side + gap(side) + text
        );

        let bare: Checkbox<()> = Checkbox::inert("");
        assert_eq!(bare.preferred_width(&theme::DARK, &mut engine), side);
    }

    /// Assigning is not the same as somebody clicking. A `set_checked` that
    /// emitted would come straight back to an application that had just handled
    /// the message it was reacting to.
    #[test]
    fn setting_the_value_programmatically_is_silent() {
        let mut checkbox: Checkbox<bool> = Checkbox::new("Mute", |on| on);
        assert!(!checkbox.checked());
        checkbox.set_checked(true);
        assert!(checkbox.checked());
        // Nothing to assert about messages here: `set_checked` has no way to emit
        // one. That is the point, and this test exists so that giving it one
        // would have to be a deliberate change to a signature.
    }
}
