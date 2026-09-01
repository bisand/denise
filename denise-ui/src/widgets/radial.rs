//! A ring that fills clockwise, with room for a number in the middle.

use alloc::string::{String, ToString};

use denise::{Point, Rect, Role};
use denise_render::{Canvas, TURN};
use denise_text::TextStyle;

use crate::widget::{PaintCtx, Widget};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Property, PropertyKind, ROLES, Value,
};
use crate::widgets::style::{Align, draw_aligned, interactive_pair};

/// A determinate circular progress indicator, `0.0` to `1.0`.
///
/// Not interactive, not focusable, not a tab stop. It reports; it does not take
/// input. The value contract is [`Progress`](super::Progress)'s exactly — NaN
/// draws an empty ring, out of range clamps, and [`update`](RadialProgress::update)
/// reports whether the value actually moved — because the number is `done / total`
/// here too, and `total` is eventually zero.
///
/// ```
/// # use denise_ui::RadialProgress;
/// # use denise::theme::Role;
/// # let (used, capacity) = (7.0f32, 10.0f32);
/// RadialProgress::new(0.7).with_label("70 %");
/// RadialProgress::new(used / capacity).with_role(Role::Warning);
/// ```
///
/// # It is a circle, so it inscribes rather than fills
///
/// The ring takes `min(width, height) / 2` as its radius and centres itself in
/// the rectangle. A radial progress squashed into an ellipse is not a radial
/// progress, and the alternative — demanding a square — would make every caller
/// do this arithmetic instead.
///
/// # It takes a label, and `Progress` deliberately does not
///
/// Not an inconsistency. A percentage drawn inside a *bar* sits on the fill at
/// one end and on the track at the other, so it needs two colours in one string
/// to stay readable; that is why [`Progress`](super::Progress) has no text.
///
/// A ring has an empty middle. The label sits on the panel behind the widget
/// rather than on the ring, so it is one colour — the same arrangement
/// [`Divider`](super::Divider)'s label already uses. The two-colour problem
/// simply does not arise, which is the whole reason this is where the number
/// goes.
///
/// The caller formats it. A widget that turned `0.7` into `"70 %"` would be
/// choosing decimals and a locale on the caller's behalf, and
/// [`update`](RadialProgress::update) plus [`set_label`](RadialProgress::set_label)
/// are two calls in the place that already knows both.
#[derive(Clone, Debug)]
pub struct RadialProgress {
    value: f32,
    label: String,
    role: Role,
    /// `None` derives it from the radius; see [`thickness_for`].
    thickness: Option<i32>,
    style: TextStyle,
}

impl RadialProgress {
    /// A ring at `value`, which is clamped — see [`RadialProgress::set_value`].
    pub fn new(value: f32) -> Self {
        Self {
            value: clamp(value),
            label: String::new(),
            role: Role::Primary,
            thickness: None,
            style: TextStyle::built_in(16),
        }
    }

    /// Puts text in the middle of the ring.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Sets the colour of the filled arc.
    ///
    /// `Warning` or `Error` for a ring that means something is running out
    /// rather than something is being achieved.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Sets the ring's thickness in pixels, instead of deriving it.
    ///
    /// Clamped to the radius: a thickness at or past it fills to the centre,
    /// which is a pie chart rather than a ring and leaves no hole for a label.
    pub fn with_thickness(mut self, thickness: i32) -> Self {
        self.thickness = Some(thickness);
        self
    }

    /// Sets the label's font and size.
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// The current value, always in `0.0..=1.0`.
    #[inline]
    pub const fn value(&self) -> f32 {
        self.value
    }

    /// Sets the value, clamped into range, with NaN as zero.
    ///
    /// The reasoning is [`Progress::set_value`](super::Progress::set_value)'s: a
    /// panic inside a paint loop on a kiosk is a black screen with no way to
    /// report itself, so a nonsense number draws an honest empty ring instead.
    pub fn set_value(&mut self, value: f32) {
        self.value = clamp(value);
    }

    /// Sets the value, reporting whether it actually changed.
    ///
    /// A panel writes its readings every cycle whether or not they moved, and
    /// repainting for a value that did not change is how an idle device stops
    /// being idle.
    pub fn update(&mut self, value: f32) -> bool {
        let value = clamp(value);
        let changed = value != self.value;
        self.value = value;
        changed
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

    /// Replaces the label, reporting whether it actually changed.
    pub fn update_label(&mut self, label: &str) -> bool {
        let changed = self.label != label;
        if changed {
            self.label = label.to_string();
        }
        changed
    }

    /// Replaces the colour role.
    pub fn set_role(&mut self, role: Role) {
        self.role = role;
    }

    /// Replaces the label's font and size.
    pub fn set_style(&mut self, style: TextStyle) {
        self.style = style;
    }
}

impl Default for RadialProgress {
    fn default() -> Self {
        Self::new(0.0)
    }
}

/// Into `0.0..=1.0`, with NaN as zero.
///
/// `f32::clamp` alone will not do: it propagates NaN rather than choosing an
/// end, so a `0.0 / 0.0` would reach the arc as a sweep of NaN.
#[inline]
fn clamp(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

/// The largest circle centred in `bounds`: its centre and radius.
pub(crate) fn ring(bounds: Rect) -> (Point, i32) {
    let radius = bounds.width.min(bounds.height) / 2;
    let centre = Point::new(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
    (centre, radius.max(0))
}

/// The ring's thickness: what was asked for, or a fifth of the radius.
///
/// A fifth is chunky enough to read across a room and thin enough to leave a
/// hole a number fits in. Clamped to the radius either way, because a thicker
/// ring than that is a filled disc — [`Canvas::stroke_arc`] draws it happily,
/// but a label would then have nowhere to go.
pub(crate) fn thickness_for(radius: i32, requested: Option<i32>) -> i32 {
    let limit = radius.max(1);
    match requested {
        Some(thickness) => thickness.clamp(1, limit),
        None => (radius / 5).clamp(1, limit),
    }
}

/// The track's colour and the moving arc's, for a ring in `role`.
///
/// One function because [`Spinner`](super::Spinner) needs exactly the same
/// answer, and because the disabled case is not obvious. `interactive_pair`
/// recesses **every** role to `Base200` when disabled, so a disabled ring would
/// draw its arc in the same colour as its track and lose its value entirely —
/// the mistake [`RadioGroup`](super::RadioGroup) avoided by keeping a mark in
/// its disabled disc, and [`List`](super::List) by giving its disabled
/// selection `Base300`. Same answer here, and found the same way: by looking at
/// the rendered showcase.
pub(crate) fn ring_colors(
    theme: &denise::Theme,
    state: crate::widget::VisualState,
    role: Role,
) -> (denise::Color, denise::Color) {
    let (track, _) = interactive_pair(theme, Role::Base300, state);
    if state.contains(crate::widget::VisualState::DISABLED) {
        // The theme's own next step up from the recessed surface: still plainly
        // disabled, still plainly showing how far round it got.
        (track, theme.color(Role::Base300))
    } else {
        (track, interactive_pair(theme, role, state).0)
    }
}

/// The sweep for `value` on a ring of `radius`, in [`TURN`] units.
///
/// The ends are exact: zero draws nothing and one draws the whole ring, which
/// costs no special case because a sweep of `TURN` *is* the circle.
///
/// Between them there is a floor, for [`Progress`](super::Progress)'s reason —
/// a job that has started must not look like a job that has not. The bar's floor
/// is one pixel; a ring's has to come from its radius, because the same angle is
/// a different arc length on a ring of 8 pixels and one of 200. The floor here
/// is the sweep whose arc is about one pixel long: `TURN / 2πr`, with 6 stood in
/// for 2π so the answer errs generous rather than invisible.
fn sweep_of(value: f32, radius: i32) -> i32 {
    // `is_nan` is not redundant: every comparison with NaN is false, so a NaN
    // would fall past both guards and reach `as i32`.
    if value.is_nan() || value <= 0.0 {
        return 0;
    }
    if value >= 1.0 {
        return TURN;
    }
    let exact = (value * TURN as f32) as i32;
    let floor = (TURN / (6 * radius.max(1))).max(1);
    exact.clamp(floor, TURN)
}

impl<M: 'static> Widget<M> for RadialProgress {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        let bounds = ctx.bounds;
        if bounds.is_empty() {
            return;
        }
        let (centre, radius) = ring(bounds);
        if radius <= 0 {
            return;
        }
        let thickness = thickness_for(radius, self.thickness);

        // The track, then the value over it. Both are surfaces drawn on the
        // panel rather than content drawn on each other, so they take the
        // surface half of their own pairing — `Progress` does the same.
        let (track, fill) = ring_colors(ctx.theme, ctx.state, self.role);
        canvas.stroke_circle(centre, radius, thickness, track);

        let sweep = sweep_of(self.value, radius);
        if sweep > 0 {
            // From twelve o'clock, clockwise: the direction every clock and
            // every progress ring agrees on.
            canvas.stroke_arc(centre, radius, thickness, 0, sweep, fill);
        }

        if self.label.is_empty() {
            return;
        }
        // Centred in the bounds, which is centred in the hole — the ring is
        // concentric with its rectangle, so there is nothing a separate hole-
        // sized box would move. An earlier version computed the square
        // inscribed in the hole and centred in *that*, which is the same
        // pixels; a mutation swapping one for the other changed nothing, which
        // is how it came out.
        //
        // A label wider than the hole therefore overflows onto the ring rather
        // than being cut off. That is the better failure: an overlap is visible
        // to whoever set the text, where a truncated "100 %" reading "10" is
        // not. Size the text down, or give the ring a thinner band.
        let content = interactive_pair(ctx.theme, Role::Base100, ctx.state).1;
        draw_aligned(
            canvas,
            ctx.text,
            self.style,
            bounds,
            (Align::Center, Align::Center),
            &self.label,
            content,
        );
    }
}

impl Describe for RadialProgress {
    const KIND: &'static str = "radial-progress";
    const DOC: &'static str = "A ring that fills clockwise, with room for a number inside it.";
    const GROUP: Group = Group::Indicator;
    const ICON: &'static denise_render::icon::Icon = &super::icons::RADIAL_PROGRESS;

    const PROPERTIES: &'static [Property] = &[
        Property::new(
            "value",
            PropertyKind::Float { min: 0.0, max: 1.0 },
            "How far round the ring is filled, from empty at `0.0` to a full turn at `1.0`.",
        ),
        Property::new(
            "label",
            PropertyKind::Text,
            "Text drawn in the middle of the ring; the caller formats it, so the widget chooses neither decimals nor a locale.",
        ),
        Property::new(
            "thickness",
            PropertyKind::Int { min: 1, max: 64 },
            "Ring width in pixels; derived from the radius without it.",
        )
        .in_pixels(),
        Property::new(
            "role",
            PropertyKind::Enum(ROLES),
            "Colour of the filled arc; `warning` or `error` for a ring that means something is running out.",
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
            "value" => Value::Float(self.value),
            // An empty string is how this widget spells "no label", so there is
            // nothing to report and nothing for a file to write.
            "label" if !self.label.is_empty() => Value::text(self.label.as_str()),
            "thickness" => Value::Int(self.thickness?),
            "role" => Value::role(self.role),
            "size" => Value::Int(i32::from(self.style.size_px)),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            // Through the setter, which is where NaN and out of range are
            // decided; the range above is only what an inspector offers.
            "value" => self.set_value(value.as_float()?),
            "label" => self.set_label(value.as_text()?),
            // Stored as asked for and clamped to the radius at paint time by
            // `thickness_for`, because the radius is not known until then.
            "thickness" => self.thickness = Some(value.as_int()?),
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

    /// The ends are exact. A ring that never quite closes at 100% is a control
    /// that disagrees with the number beside it — and it costs no special case,
    /// because a sweep of `TURN` is the circle.
    #[test]
    fn the_sweep_is_exact_at_both_ends() {
        for radius in [4, 20, 200] {
            assert_eq!(sweep_of(0.0, radius), 0, "radius {radius}");
            assert_eq!(sweep_of(1.0, radius), TURN, "radius {radius}");
        }
        assert_eq!(sweep_of(0.5, 100), TURN / 2);
        assert_eq!(sweep_of(0.25, 100), TURN / 4);
    }

    /// `done / total` with a zero total is the case this exists for.
    #[test]
    fn a_value_that_is_not_a_number_draws_an_empty_ring() {
        let done = core::hint::black_box(0.0f32);
        let total = core::hint::black_box(0.0f32);
        let zero_over_zero = done / total;
        assert!(zero_over_zero.is_nan(), "the premise");
        assert_eq!(clamp(zero_over_zero), 0.0);
        assert_eq!(sweep_of(zero_over_zero, 40), 0);

        let mut ring = RadialProgress::new(0.5);
        ring.set_value(zero_over_zero);
        assert_eq!(ring.value(), 0.0, "and it does not keep the old value");
    }

    /// Infinity is a direction rather than a mistake.
    #[test]
    fn infinities_clamp_to_the_end_they_point_at() {
        assert_eq!(clamp(f32::INFINITY), 1.0);
        assert_eq!(clamp(f32::NEG_INFINITY), 0.0);
        assert_eq!(sweep_of(f32::INFINITY, 40), TURN);
        assert_eq!(sweep_of(f32::NEG_INFINITY, 40), 0);
    }

    /// Out of range clamps rather than panicking inside a paint loop.
    #[test]
    fn values_outside_the_range_are_clamped() {
        assert_eq!(RadialProgress::new(42.0).value(), 1.0);
        assert_eq!(RadialProgress::new(-42.0).value(), 0.0);
    }

    /// A ring that has barely started must not look like one that has not, and
    /// the floor that guarantees it has to scale with the radius: the same angle
    /// is a different arc length on a ring of 8 pixels and one of 200.
    #[test]
    fn a_value_just_above_zero_shows_an_arc_at_any_radius() {
        for radius in [4, 8, 20, 60, 200] {
            let tiny = sweep_of(0.000_01, radius);
            assert!(tiny > 0, "radius {radius} showed nothing");
            // About a pixel of arc: length is r·θ, θ = sweep/TURN · 2π.
            let length = radius as f32 * (tiny as f32 / TURN as f32) * core::f32::consts::TAU;
            assert!(
                (0.8..4.0).contains(&length),
                "radius {radius}: floor is {length} pixels of arc"
            );
        }
    }

    /// Monotonic, and never past a full turn.
    #[test]
    fn more_progress_is_never_less_sweep() {
        for radius in [4, 40, 200] {
            let mut previous = -1;
            for step in 0..=1000 {
                let sweep = sweep_of(step as f32 / 1000.0, radius);
                assert!(sweep >= previous, "radius {radius} went back at {step}");
                assert!(sweep <= TURN, "radius {radius} overran at {step}");
                previous = sweep;
            }
        }
    }

    /// The ring inscribes itself: a wide rectangle gives a circle of the short
    /// side, centred, not an ellipse and not something that escapes.
    #[test]
    fn the_ring_inscribes_itself_in_any_rectangle() {
        for bounds in [
            Rect::new(0, 0, 100, 40),
            Rect::new(10, 20, 40, 100),
            Rect::new(-5, -5, 60, 60),
            Rect::new(0, 0, 1, 1),
        ] {
            let (centre, radius) = ring(bounds);
            assert_eq!(radius, bounds.width.min(bounds.height) / 2, "{bounds:?}");
            let circle = Rect::new(centre.x - radius, centre.y - radius, radius * 2, radius * 2);
            assert!(
                bounds.contains_rect(&circle),
                "{bounds:?}: the ring {circle:?} escaped"
            );
        }
    }

    /// The thickness never exceeds the radius, however absurd the request — past
    /// that it is a filled disc with nowhere to put a label.
    #[test]
    fn the_thickness_never_exceeds_the_radius() {
        for radius in [0, 1, 2, 5, 20, 200] {
            for requested in [None, Some(-5), Some(0), Some(1), Some(9_999)] {
                let t = thickness_for(radius, requested);
                assert!(t >= 1, "radius {radius} {requested:?} gave {t}");
                assert!(t <= radius.max(1), "radius {radius} {requested:?} gave {t}");
            }
        }
        assert_eq!(thickness_for(50, None), 10, "a fifth of the radius");
        assert_eq!(thickness_for(50, Some(4)), 4);
    }

    /// A panel writes its readings every cycle whether or not they moved.
    #[test]
    fn writing_the_same_value_or_label_reports_no_change() {
        let mut ring = RadialProgress::new(0.0);
        assert!(ring.update(0.4));
        assert!(!ring.update(0.4));
        assert!(ring.update(0.6));

        assert!(ring.update_label("60 %"));
        assert!(!ring.update_label("60 %"));
        assert!(ring.update_label("61 %"));
        assert_eq!(ring.label(), "61 %");
    }

    /// A disabled ring still shows how far round it got. `interactive_pair`
    /// recesses every role to the same `Base200` when disabled, so this needs
    /// its own answer — the same lesson `RadioGroup` learned about its disabled
    /// disc and `List` about its disabled selection.
    #[test]
    fn a_disabled_ring_still_shows_its_value() {
        use crate::widget::VisualState;
        use denise::Theme;

        for theme in Theme::BUILT_IN {
            for role in [Role::Primary, Role::Warning, Role::Success] {
                let (track, arc) = ring_colors(&theme, VisualState::DISABLED, role);
                assert_ne!(
                    track, arc,
                    "{} {role:?}: a disabled ring lost its value",
                    theme.name
                );
                // And an enabled one is still obviously coloured.
                let (track, arc) = ring_colors(&theme, VisualState::NONE, role);
                assert_ne!(track, arc, "{} {role:?} enabled", theme.name);
            }
        }
    }

    /// The label has to stay readable on the panel behind it, in every theme and
    /// state — the floor every widget here is held to.
    #[test]
    fn the_label_is_readable_on_the_panel_in_every_theme() {
        use crate::widget::VisualState;
        use denise::Theme;
        use denise::theme::{AA_LARGE, contrast_x100};

        for theme in Theme::BUILT_IN {
            for state in [VisualState::NONE, VisualState::DISABLED] {
                let (surface, content) = interactive_pair(&theme, Role::Base100, state);
                let ratio = contrast_x100(surface, content);
                assert!(
                    ratio >= AA_LARGE,
                    "{} {state:?}: label on the panel is {ratio}, floor is {AA_LARGE}",
                    theme.name
                );
            }
        }
    }
}
