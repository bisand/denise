//! A line, optionally with a label in the middle.

use alloc::string::String;

use denise::{Point, Rect, Role};
use denise::Pen;
use denise_text::TextStyle;

use crate::widget::{PaintCtx, Widget};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, ORIENTATIONS, Property, PropertyKind, ROLES, Value,
};
use crate::widgets::style::{Orientation, interactive_pair};

/// A rule between two groups of content.
///
/// The smallest widget here, and worth having only because everybody draws it
/// slightly differently otherwise — one pixel or two, `Base300` or a faded
/// `BaseContent`, inset or full width. It is here so a panel is consistent with
/// itself.
///
/// Not interactive, not focusable, no messages.
///
/// # A label only makes sense across
///
/// A horizontal divider with a label draws line, gap, text, gap, line. A
/// **vertical** one ignores its label and draws an unbroken rule, because the
/// text would have to be rotated and there is no rotated text in the rasteriser.
/// Ignoring it is better than drawing horizontal text through a vertical line and
/// calling that a feature.
///
/// # About "one pixel"
///
/// The thickness comes from [`Metrics::border`](denise::theme::Metrics::border),
/// which is a *logical* pixel — one at scale factor 1, two under
/// [`Metrics::TOUCH`], and whatever the application's scale factor makes of it
/// on a dense display: a scale-aware application passes
/// `theme.scaled(factor)` at construction and this widget's rule thickens with
/// everything else. See `docs/design.md` for the pattern.
///
/// [`Metrics::TOUCH`]: denise::theme::Metrics::TOUCH
#[derive(Clone, Debug)]
pub struct Divider {
    label: String,
    orientation: Orientation,
    role: Role,
    style: TextStyle,
}

impl Divider {
    /// A horizontal rule with no label.
    pub fn new() -> Self {
        Self {
            label: String::new(),
            orientation: Orientation::Horizontal,
            role: Role::Base300,
            style: TextStyle::built_in(16),
        }
    }

    /// A horizontal rule with `label` in the middle.
    pub fn labelled(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ..Self::new()
        }
    }

    /// A vertical rule. Any label is ignored — see the type documentation.
    pub fn vertical() -> Self {
        Self {
            orientation: Orientation::Vertical,
            ..Self::new()
        }
    }

    /// Sets the line's colour role.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Sets the label's font and size.
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// The current label, empty if there is none.
    #[inline]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Replaces the label.
    pub fn set_label(&mut self, label: impl Into<String>) {
        self.label = label.into();
    }

    /// Replaces the label's font and size.
    pub fn set_style(&mut self, style: TextStyle) {
        self.style = style;
    }

    /// Which way it runs.
    #[inline]
    pub const fn orientation(&self) -> Orientation {
        self.orientation
    }
}

impl Default for Divider {
    fn default() -> Self {
        Self::new()
    }
}

/// Space between the rule and the label at each side.
#[inline]
const fn gap(size_px: u16) -> i32 {
    // `Ord::max` is not const yet.
    let half = size_px as i32 / 2;
    if half < 1 { 1 } else { half }
}

/// The two line segments and the text box between them.
///
/// `None` for the text box when there is no label, or when the label plus its
/// gaps would leave no room for a rule on either side — at which point drawing a
/// two-pixel stub each end says less than an unbroken line does. That is the
/// "degrades sensibly" case: the label still draws, and it draws over the whole
/// width rather than between two fragments.
fn layout(bounds: Rect, thickness: i32, text_width: i32, gap: i32) -> (Rect, Option<Rect>, Rect) {
    let y = bounds.y + (bounds.height - thickness) / 2;
    let full = Rect::new(bounds.x, y, bounds.width, thickness);
    if text_width <= 0 {
        return (full, None, Rect::new(bounds.right(), y, 0, thickness));
    }

    // A rule is worth drawing only if there is a visible amount of it. Below
    // this the label takes the whole width.
    const LEAST_RULE: i32 = 8;
    let side = (bounds.width - text_width - gap * 2) / 2;
    if side < LEAST_RULE {
        return (
            Rect::new(bounds.x, y, 0, thickness),
            Some(bounds),
            Rect::new(bounds.right(), y, 0, thickness),
        );
    }

    let left = Rect::new(bounds.x, y, side, thickness);
    let text = Rect::new(bounds.x + side + gap, bounds.y, text_width, bounds.height);
    // Computed from the right edge rather than from the left segment's width, so
    // the two ends are equal even when the odd pixel of the division has to go
    // somewhere.
    let right_x = bounds.right() - side;
    let right = Rect::new(right_x, y, side, thickness);
    (left, Some(text), right)
}

impl<M: 'static> Widget<M> for Divider {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        let bounds = ctx.bounds;
        if bounds.is_empty() {
            return;
        }
        let thickness = ctx.theme.metrics.border.max(1);
        let line = ctx.theme.color(self.role);

        if self.orientation == Orientation::Vertical {
            let x = bounds.x + (bounds.width - thickness) / 2;
            canvas.fill_rect(Rect::new(x, bounds.y, thickness, bounds.height), line);
            return;
        }

        if self.label.is_empty() {
            let y = bounds.y + (bounds.height - thickness) / 2;
            canvas.fill_rect(Rect::new(bounds.x, y, bounds.width, thickness), line);
            return;
        }

        // Measured through the engine rather than guessed from the character
        // count, so the gaps are right with a proportional font.
        let text_width = ctx.text.measure_line(self.style, &self.label);
        let (left, text, right) = layout(bounds, thickness, text_width, gap(self.style.size_px));

        if left.width > 0 {
            canvas.fill_rect(left, line);
        }
        if right.width > 0 {
            canvas.fill_rect(right, line);
        }
        let Some(text_bounds) = text else {
            return;
        };

        let extent = ctx.text.measure(self.style, &self.label);
        let at = Point::new(
            text_bounds.x + (text_bounds.width - extent.width as i32) / 2,
            text_bounds.y + (text_bounds.height - extent.height as i32) / 2,
        );
        // The label is content on the panel, not part of the rule, so it takes
        // the base pairing and mutes with the rest of a disabled group.
        let content = interactive_pair(ctx.theme, Role::Base100, ctx.state).1;
        ctx.text.draw(canvas, self.style, at, &self.label, content);
    }
}

impl Describe for Divider {
    const KIND: &'static str = "divider";
    const DOC: &'static str = "A line between things, with an optional label in the middle.";
    const GROUP: Group = Group::Display;
    const ICON: &'static denise::icon::Icon = &super::icons::DIVIDER;

    const PROPERTIES: &'static [Property] = &[
        Property::new(
            "label",
            PropertyKind::Text,
            "An optional label sitting in the rule.",
        ),
        Property::new(
            "orientation",
            PropertyKind::Enum(ORIENTATIONS),
            "Which way the rule runs.",
        ),
        Property::new("role", PropertyKind::Enum(ROLES), "The rule's colour."),
        Property::new(
            "size",
            PropertyKind::Int { min: 6, max: 96 },
            "Text size in logical pixels; only a labelled divider draws text.",
        )
        .in_pixels(),
    ];

    fn get(&self, name: &str) -> Option<Value> {
        Some(match name {
            // An empty label *is* the absence of one — the field is a `String`
            // rather than an `Option` because painting treats the two the same —
            // so an unlabelled divider reports nothing and writes nothing.
            "label" if self.label.is_empty() => return None,
            "label" => Value::text(self.label.as_str()),
            "orientation" => Value::orientation(self.orientation),
            "role" => Value::role(self.role),
            "size" => Value::Int(i32::from(self.style.size_px)),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            "label" => self.label = value.as_text()?,
            "orientation" => self.orientation = value.as_orientation()?,
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

    const THICKNESS: i32 = 1;
    const GAP: i32 = 8;

    /// The label sits in the middle and the two rules are the same length. An
    /// off-centre label is the one thing anybody would notice about a divider.
    #[test]
    fn the_two_segments_are_equal_and_the_label_is_centred() {
        // An odd leftover on purpose: 201 wide, 40 of text, 8 gaps — the halves
        // cannot both be whole, and the pixel has to go somewhere invisible.
        let bounds = Rect::new(10, 4, 201, 24);
        let (left, text, right) = layout(bounds, THICKNESS, 40, GAP);
        let text = text.expect("a labelled divider has a text box");

        assert_eq!(
            left.width, right.width,
            "one side is longer than the other: {left:?} {right:?}"
        );
        assert_eq!(left.x, bounds.x, "the left rule starts at the left edge");
        assert_eq!(
            right.right(),
            bounds.right(),
            "and the right one ends at the right edge"
        );
        assert!(text.x > left.right(), "the label overlaps the left rule");
        assert!(text.right() < right.x, "the label overlaps the right rule");
    }

    /// The rules and the label are vertically centred on the same line.
    #[test]
    fn the_rule_is_centred_in_the_height_it_is_given() {
        let bounds = Rect::new(0, 100, 300, 30);
        let (left, _, right) = layout(bounds, 2, 40, GAP);
        assert_eq!(left.y, right.y);
        assert_eq!(left.height, 2, "the thickness is what it was given");
        let above = left.y - bounds.y;
        let below = bounds.bottom() - left.bottom();
        assert_eq!(
            above, below,
            "the rule is not centred: {above} above, {below} below"
        );
    }

    /// A label wider than the space degrades to a label on its own rather than
    /// to two stubs, or to a rule drawn under the text.
    #[test]
    fn a_label_too_wide_for_its_divider_takes_the_whole_width() {
        let bounds = Rect::new(0, 0, 60, 24);
        let (left, text, right) = layout(bounds, THICKNESS, 200, GAP);
        assert_eq!(left.width, 0, "no stub on the left");
        assert_eq!(right.width, 0, "nor on the right");
        assert_eq!(text, Some(bounds), "the label gets the whole rectangle");
    }

    /// And the boundary between the two behaviours is not a cliff into negative
    /// widths.
    #[test]
    fn every_width_produces_segments_with_sane_geometry() {
        for width in 0..240 {
            let bounds = Rect::new(3, 0, width, 20);
            let (left, text, right) = layout(bounds, THICKNESS, 40, GAP);
            assert!(left.width >= 0, "width {width}: negative left rule");
            assert!(right.width >= 0, "width {width}: negative right rule");
            if left.width > 0 {
                let text = text.expect("segments imply a text box");
                assert!(
                    left.right() <= text.x && text.right() <= right.x,
                    "width {width}: the pieces overlap"
                );
            }
        }
    }

    /// With no label there is one unbroken rule across the whole width.
    #[test]
    fn an_unlabelled_divider_is_one_unbroken_rule() {
        let bounds = Rect::new(5, 5, 120, 10);
        let (left, text, right) = layout(bounds, THICKNESS, 0, GAP);
        assert_eq!(left.width, bounds.width);
        assert_eq!(left.x, bounds.x);
        assert_eq!(text, None);
        assert_eq!(right.width, 0);
    }

    /// The constructors say what they build, so a vertical divider cannot be
    /// mistaken for a horizontal one that happens to be narrow.
    #[test]
    fn the_constructors_pick_the_orientation_and_the_label() {
        assert_eq!(Divider::new().orientation(), Orientation::Horizontal);
        assert_eq!(Divider::vertical().orientation(), Orientation::Vertical);
        assert_eq!(Divider::labelled("eller").label(), "eller");
        assert_eq!(Divider::new().label(), "");
        assert_eq!(Divider::default().orientation(), Orientation::Horizontal);
    }
}
