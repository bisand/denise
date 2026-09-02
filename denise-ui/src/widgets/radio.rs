//! One choice from a few, and the keyboard rules that make it one.

use alloc::string::String;
use alloc::vec::Vec;

use denise::{ElementState, InputEvent, KeyCode, Point, Rect, Role, Theme};
use denise_render::Pen;
use denise_text::{TextEngine, TextStyle};

use crate::widget::{
    Event, EventCtx, Handled, MeasureCtx, Measured, Offer, PaintCtx, VisualState, Widget,
};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Payload, Property, PropertyKind, ROLES, Value,
};
use crate::widgets::style::{Align, draw_aligned, focus_ring, interactive_pair};

/// A set of options, exactly one of which is chosen.
///
/// **The widget is the group, not the button.** A lone radio is a checkbox that
/// cannot be unchecked; the exclusivity is the entire point, and it is what
/// decides the shape here:
///
/// - The selection is one index, so *two selected* and *none selected* are both
///   unrepresentable rather than merely avoided.
/// - The group is **one node**, so it is **one tab stop**. Tab moves past the
///   whole thing, which is the behaviour applications assembling their own radios
///   out of separate widgets almost always get wrong.
/// - Arrow keys move within it and wrap, selecting as they go — the convention
///   every platform shares, and the reason a keyboard-only panel can drive this.
///
/// Options are laid out vertically, dividing the bounds evenly. The message is a
/// function of the newly chosen index:
///
/// ```
/// # use denise_ui::RadioGroup;
/// enum Message { Mode(usize) }
/// RadioGroup::new(["Auto", "Manual", "Off"], Message::Mode);
/// ```
#[derive(Clone, Debug)]
pub struct RadioGroup<M> {
    options: Vec<String>,
    selected: usize,
    message: Option<fn(usize) -> M>,
    role: Role,
    style: TextStyle,
}

impl<M> RadioGroup<M> {
    /// A group with the first option chosen.
    pub fn new(
        options: impl IntoIterator<Item = impl Into<String>>,
        message: fn(usize) -> M,
    ) -> Self {
        Self {
            options: options.into_iter().map(Into::into).collect(),
            selected: 0,
            message: Some(message),
            role: Role::Primary,
            style: TextStyle::built_in(16),
        }
    }

    /// A group that emits nothing, for a choice the application reads rather
    /// than reacts to.
    pub fn inert(options: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            options: options.into_iter().map(Into::into).collect(),
            selected: 0,
            message: None,
            role: Role::Primary,
            style: TextStyle::built_in(16),
        }
    }

    /// Sets the initially chosen option. Out of range chooses the last one.
    pub fn with_selected(mut self, index: usize) -> Self {
        self.selected = self.clamp(index);
        self
    }

    /// Sets the colour role of the chosen option.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Sets the labels' font and size.
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets the labels' size, keeping the font.
    pub fn with_size(mut self, size_px: u16) -> Self {
        self.style.size_px = size_px;
        self
    }

    /// The chosen index. Always in range while the group has options.
    #[inline]
    pub const fn selected(&self) -> usize {
        self.selected
    }

    /// The chosen option's text, or `None` for an empty group.
    #[inline]
    pub fn selected_label(&self) -> Option<&str> {
        self.options.get(self.selected).map(String::as_str)
    }

    /// Chooses an option **without emitting anything**. Out of range is clamped.
    ///
    /// Silent for the same reason [`Checkbox::set_checked`] is: the message
    /// reports what a person did, and an application that assigned here and got
    /// its own message back would either loop or have to guard against itself.
    ///
    /// [`Checkbox::set_checked`]: super::Checkbox::set_checked
    pub fn set_selected(&mut self, index: usize) {
        self.selected = self.clamp(index);
    }

    /// The options, in order.
    #[inline]
    pub fn options(&self) -> &[String] {
        &self.options
    }

    /// Replaces the options, keeping the chosen index in range.
    ///
    /// A shorter list can leave `selected` past the end, and a group pointing at
    /// an option that no longer exists would draw nothing as chosen — which is
    /// the one state this widget exists to make impossible.
    pub fn set_options(&mut self, options: impl IntoIterator<Item = impl Into<String>>) {
        self.options = options.into_iter().map(Into::into).collect();
        self.selected = self.clamp(self.selected);
    }

    /// Replaces the colour role.
    pub fn set_role(&mut self, role: Role) {
        self.role = role;
    }

    /// Replaces the labels' font and size.
    pub fn set_style(&mut self, style: TextStyle) {
        self.style = style;
    }

    /// Width the widest option needs, circle and gap included.
    pub fn preferred_width(&self, theme: &Theme, engine: &mut TextEngine) -> i32 {
        let side = theme.metrics.size_selector;
        let widest = self
            .options
            .iter()
            .map(|option| engine.measure_line(self.style, option))
            .max()
            .unwrap_or(0);
        if widest == 0 {
            side
        } else {
            side + gap(side) + widest
        }
    }

    /// Height this group needs to give every option a comfortable row.
    ///
    /// Rows divide the bounds evenly, so a caller that gives it less gets
    /// tighter rows rather than a clipped list — this is the size at which the
    /// circles are not touching each other.
    pub fn preferred_height(&self, theme: &Theme) -> i32 {
        let row = theme.metrics.size_selector * 3 / 2;
        row * self.options.len().max(1) as i32
    }

    /// An index that exists, or zero for an empty group.
    #[inline]
    fn clamp(&self, index: usize) -> usize {
        index.min(self.options.len().saturating_sub(1))
    }

    /// Moves the selection by one, wrapping.
    ///
    /// Wrapping is right *here* and wrong for a list: a radio group is a handful
    /// of options a person can see all at once, so coming round from the last to
    /// the first is obvious. A hundred-row list doing the same is disorienting,
    /// which is why #16 stops at the ends instead.
    fn step(&self, forward: bool) -> usize {
        let count = self.options.len();
        if count == 0 {
            return 0;
        }
        if forward {
            (self.selected + 1) % count
        } else {
            (self.selected + count - 1) % count
        }
    }
}

/// Space between a circle and its label.
#[inline]
const fn gap(side: i32) -> i32 {
    if side < 2 { 1 } else { side / 2 }
}

/// The rectangle for one option.
///
/// Edges are computed from the index rather than accumulated, so `count` rows
/// tile the bounds exactly however the division rounds — an accumulated row
/// height leaves a gap at the bottom that grows with the option count.
fn row_rect(bounds: Rect, count: usize, index: usize) -> Rect {
    if count == 0 {
        return Rect::new(bounds.x, bounds.y, bounds.width, 0);
    }
    // Widened: `height * index` overflows an `i32` for a rectangle a caller is
    // entitled to pass, and this is arithmetic on somebody else's numbers.
    let edge = |i: usize| -> i32 {
        (i64::from(bounds.height) * i as i64 / count as i64) as i32 + bounds.y
    };
    Rect::from_edges(bounds.x, edge(index), bounds.right(), edge(index + 1))
}

/// Which option contains `point`, if any.
fn row_at(bounds: Rect, count: usize, point: Point) -> Option<usize> {
    if count == 0 || !bounds.contains(point) {
        return None;
    }
    let offset = i64::from(point.y - bounds.y);
    let index = (offset * count as i64 / i64::from(bounds.height.max(1))) as usize;
    Some(index.min(count - 1))
}

/// The circle for one option: a square at the leading edge, centred in its row.
fn circle_rect(row: Rect, theme: &Theme) -> Rect {
    let side = theme
        .metrics
        .size_selector
        .min(row.height)
        .min(row.width)
        .max(1);
    Rect::new(row.x, row.y + (row.height - side) / 2, side, side)
}

impl<M: 'static> Widget<M> for RadioGroup<M> {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn measure(&self, ctx: &mut MeasureCtx<'_>, _offered: Offer) -> Measured {
        Measured::both(
            self.preferred_width(ctx.theme, ctx.text),
            self.preferred_height(ctx.theme),
        )
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        let count = self.options.len();
        if count == 0 {
            return;
        }
        let label_color = interactive_pair(ctx.theme, Role::Base100, ctx.state).1;

        for (index, option) in self.options.iter().enumerate() {
            let row = row_rect(ctx.bounds, count, index);
            if row.is_empty() {
                continue;
            }
            let circle = circle_rect(row, ctx.theme);
            // Radius is half the side, which the rasteriser turns into a genuine
            // antialiased circle — checked, because a "rounded rect" with flat
            // spots would make a radio look like a squashed checkbox.
            let radius = circle.width / 2;

            if index == self.selected {
                // Filled disc with a contrasting centre, mirroring `Checkbox`:
                // both colours come from one pairing, so the dot is readable on
                // every theme by construction rather than by hoping that the role
                // colour happens to contrast with `Base100`.
                let (fill, dot) = interactive_pair(ctx.theme, self.role, ctx.state);
                canvas.fill_rounded_rect(circle, radius, fill);
                // Two fifths, which leaves a small mark rather than a hole. A
                // thin ring around a large gap reads as an "O" at arm's length —
                // the shape an *unselected* radio has in most designs, which is
                // the wrong answer to be legible about. The mark also keeps the
                // disabled state readable: `interactive_pair` recesses the disc
                // to `Base200` there, and without a centre it would be nearly
                // the same drawing as an unchosen one.
                let inset = (circle.width * 2 / 5).max(1);
                let centre = circle.inflate(-inset);
                if !centre.is_empty() {
                    canvas.fill_rounded_rect(centre, centre.width / 2, dot);
                }
                if ctx.state.contains(VisualState::FOCUSED) {
                    // The ring goes round the chosen row, not the whole group:
                    // arrow keys move the choice, so the chosen option *is* the
                    // one the keyboard is pointing at.
                    focus_ring(
                        ctx.theme,
                        row,
                        ctx.theme.radius(denise::Radius::Field),
                        canvas,
                    );
                }
            } else {
                let (surface, _) = interactive_pair(ctx.theme, Role::Base100, ctx.state);
                canvas.fill_rounded_rect(circle, radius, surface);
                canvas.stroke_rounded_rect(
                    circle,
                    radius,
                    ctx.theme.metrics.border,
                    ctx.theme.color(Role::Base300),
                );
            }

            if option.is_empty() {
                continue;
            }
            let text = Rect::from_edges(
                circle.right() + gap(circle.width),
                row.y,
                row.right(),
                row.bottom(),
            );
            if !text.is_empty() {
                draw_aligned(
                    canvas,
                    ctx.text,
                    self.style,
                    text,
                    (Align::Start, Align::Center),
                    option,
                    label_color,
                );
            }
        }
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        let count = self.options.len();
        if count == 0 {
            return Handled::No;
        }

        let chosen = match event {
            Event::Input(InputEvent::PointerButton {
                state: ElementState::Up,
                position,
                ..
            }) => row_at(ctx.bounds, count, *position),
            Event::Input(InputEvent::TouchUp {
                position,
                cancelled: false,
                ..
            }) => row_at(ctx.bounds, count, *position),
            Event::Input(InputEvent::Key {
                code: code @ (KeyCode::ArrowDown | KeyCode::ArrowRight),
                state: ElementState::Down,
                ..
            })
            | Event::Input(InputEvent::Key {
                code: code @ (KeyCode::ArrowUp | KeyCode::ArrowLeft),
                state: ElementState::Down,
                ..
            }) if ctx.state.contains(VisualState::FOCUSED) => {
                let forward = matches!(code, KeyCode::ArrowDown | KeyCode::ArrowRight);
                Some(self.step(forward))
            }
            _ => return Handled::No,
        };

        let Some(chosen) = chosen else {
            return Handled::No;
        };
        if chosen == self.selected {
            // Re-choosing what is already chosen is not a change. Emitting here
            // would make an application do its work again for a click that meant
            // nothing — and consuming the event is still right, because the
            // group did handle it.
            return Handled::Yes;
        }
        self.selected = chosen;
        if let Some(message) = self.message {
            ctx.emit(message(chosen));
        }
        Handled::Yes
    }

    fn accepts_pointer(&self) -> bool {
        true
    }

    /// An empty group is not a tab stop. Focusing something with nothing in it
    /// strands a keyboard-only panel on a widget no key does anything to.
    fn focusable(&self) -> bool {
        !self.options.is_empty()
    }
}

impl<M> Describe for RadioGroup<M> {
    const KIND: &'static str = "radio-group";
    const DOC: &'static str = "One choice out of a few, all of them visible at once.";
    const GROUP: Group = Group::Input;
    const ICON: &'static denise_render::icon::Icon = &super::icons::RADIO_GROUP;

    const PROPERTIES: &'static [Property] = &[
        Property::new(
            "option",
            PropertyKind::List,
            "The choices, as `option` child nodes. A group's are the real ones: it always has an answer.",
        ),
        Property::new(
            "selected",
            PropertyKind::Int {
                min: 0,
                max: i32::MAX,
            },
            "Index into the options. A group always has an answer, so this is never unset.",
        ),
        Property::new(
            "on-change",
            PropertyKind::Message(Payload::Index),
            "Emitted with the chosen option's index.",
        ),
        Property::new(
            "role",
            PropertyKind::Enum(ROLES),
            "Colour role of the chosen option's disc.",
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
            "selected" => Value::Int(i32::try_from(self.selected).unwrap_or(i32::MAX)),
            "role" => Value::role(self.role),
            "size" => Value::Int(i32::from(self.style.size_px)),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            // Through the setter, which clamps into the options: unlike a list,
            // this widget cannot represent nothing chosen.
            "selected" => self.set_selected(value.as_index()?),
            // The engine builds these from the child nodes, and an
            // inspector edits them where they live. See
            // `PropertyKind::List`.
            "on-change" | "option" => return Err(Mismatch::Supplied),
            "role" => self.role = value.as_role()?,
            "size" => self.style.size_px = value.as_size()?,
            _ => return Err(Mismatch::Unknown),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use denise::theme;

    fn group() -> RadioGroup<usize> {
        RadioGroup::new(["Auto", "Manual", "Off"], |index| index)
    }

    /// Rows tile the bounds exactly. Computing each edge from the index rather
    /// than accumulating a row height is what makes this true at any count — the
    /// accumulated version leaves a gap at the bottom that grows with the list.
    #[test]
    fn rows_tile_the_bounds_with_no_gap_and_no_overlap() {
        for count in 1..=7 {
            let bounds = Rect::new(10, 20, 200, 101);
            let mut previous_bottom = bounds.y;
            for index in 0..count {
                let row = row_rect(bounds, count, index);
                assert_eq!(row.y, previous_bottom, "count {count}, row {index}");
                assert_eq!(row.x, bounds.x);
                assert_eq!(row.right(), bounds.right());
                previous_bottom = row.bottom();
            }
            assert_eq!(
                previous_bottom,
                bounds.bottom(),
                "count {count} left the last row short of the bottom"
            );
        }
    }

    /// Every point in the bounds belongs to exactly the row that contains it, and
    /// nothing outside belongs to any.
    #[test]
    fn a_point_lands_in_the_row_that_contains_it() {
        let bounds = Rect::new(10, 20, 200, 90);
        let count = 3;
        for index in 0..count {
            let row = row_rect(bounds, count, index);
            for y in row.y..row.bottom() {
                assert_eq!(
                    row_at(bounds, count, Point::new(bounds.x + 5, y)),
                    Some(index),
                    "y {y} should be row {index}"
                );
            }
        }
        assert_eq!(row_at(bounds, count, Point::new(5, 25)), None, "left of it");
        assert_eq!(row_at(bounds, count, Point::new(15, 5)), None, "above it");
        assert_eq!(row_at(bounds, count, Point::new(15, 500)), None, "below it");
    }

    /// A rectangle a caller is entitled to pass. `height * index` overflows an
    /// `i32` long before this, and a panic inside a paint loop on a kiosk is a
    /// black screen.
    #[test]
    fn an_absurd_rectangle_neither_overflows_nor_panics() {
        let bounds = Rect::new(0, 0, 1000, i32::MAX);
        for index in 0..4 {
            let row = row_rect(bounds, 4, index);
            assert!(row.height > 0, "row {index} of a tall group vanished");
        }
        assert!(row_at(bounds, 4, Point::new(1, i32::MAX / 2)).is_some());
    }

    /// Wrapping in both directions, which is the whole of the keyboard contract.
    #[test]
    fn arrows_wrap_in_both_directions() {
        let mut group = group();
        assert_eq!(group.selected(), 0);

        assert_eq!(group.step(true), 1);
        group.set_selected(2);
        assert_eq!(group.step(true), 0, "past the end comes back to the start");

        group.set_selected(0);
        assert_eq!(group.step(false), 2, "and before the start goes to the end");
    }

    /// A one-option group is a degenerate case that must not divide by zero or
    /// spin: every step lands on the only option there is.
    #[test]
    fn a_single_option_group_steps_to_itself() {
        let group: RadioGroup<usize> = RadioGroup::new(["Only"], |index| index);
        assert_eq!(group.step(true), 0);
        assert_eq!(group.step(false), 0);
    }

    /// An empty group has no valid index, is not a tab stop, and must not panic
    /// on any of the arithmetic that assumes one.
    #[test]
    fn an_empty_group_is_inert_rather_than_broken() {
        let mut group: RadioGroup<usize> = RadioGroup::inert(Vec::<String>::new());
        assert_eq!(group.selected(), 0);
        assert_eq!(group.selected_label(), None);
        assert_eq!(group.step(true), 0);
        assert!(!Widget::<usize>::focusable(&group));
        group.set_selected(9);
        assert_eq!(group.selected(), 0);
        assert_eq!(row_rect(Rect::new(0, 0, 10, 10), 0, 0).height, 0);
    }

    /// Out of range is clamped rather than accepted. A group pointing past its
    /// own options would draw nothing as chosen — the one state this widget
    /// exists to make impossible.
    #[test]
    fn the_selection_is_always_an_option_that_exists() {
        let mut group = group();
        group.set_selected(99);
        assert_eq!(group.selected(), 2);
        assert_eq!(group.selected_label(), Some("Off"));

        // And a shorter list drags it back into range.
        group.set_options(["One"]);
        assert_eq!(group.selected(), 0);
        assert_eq!(group.selected_label(), Some("One"));
    }

    /// The dot has to be readable inside its own disc on every theme and in every
    /// state — the same guarantee `Toggle` needed, and for the same reason: a
    /// fixed colour works in two themes out of three and ships broken in the
    /// third.
    #[test]
    fn the_dot_is_visible_inside_the_disc_in_every_theme_and_state() {
        use denise::theme::{AA_LARGE, contrast_x100};

        for theme in Theme::BUILT_IN {
            for state in [
                VisualState::NONE,
                VisualState::HOVERED,
                VisualState::PRESSED,
                VisualState::DISABLED,
                VisualState::FOCUSED,
            ] {
                let (disc, dot) = interactive_pair(&theme, Role::Primary, state);
                let ratio = contrast_x100(disc, dot);
                assert!(
                    ratio >= AA_LARGE,
                    "{} {state:?}: dot against disc is {ratio}, floor is {AA_LARGE}",
                    theme.name
                );
            }
        }
    }

    /// The circle is square at any row height, including rows too short for the
    /// theme's metric.
    #[test]
    fn the_circle_stays_square_and_inside_its_row() {
        for height in [1, 7, 20, 60] {
            let row = Rect::new(10, 20, 200, height);
            let circle = circle_rect(row, &theme::DARK);
            assert_eq!(circle.width, circle.height, "height {height}");
            assert!(circle.width >= 1, "height {height}");
            assert!(
                row.contains_rect(&circle),
                "height {height}: {circle:?} escaped {row:?}"
            );
        }
    }

    #[test]
    fn the_preferred_width_follows_the_widest_option() {
        let mut engine = TextEngine::new();
        let group = group();
        let side = theme::DARK.metrics.size_selector;
        let widest = engine.measure_line(TextStyle::built_in(16), "Manual");
        assert_eq!(
            group.preferred_width(&theme::DARK, &mut engine),
            side + gap(side) + widest
        );
    }
}
