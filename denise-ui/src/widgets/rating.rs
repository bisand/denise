//! Stars, filled to a value.

use denise::{ElementState, InputEvent, KeyCode, Point, Rect, Role};
use denise::Pen;

use crate::widget::{
    Event, EventCtx, Handled, MeasureCtx, Measured, Offer, PaintCtx, VisualState, Widget,
};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Payload, Property, PropertyKind, ROLES, Value,
};
use crate::widgets::style::{focus_ring, interactive_pair};

/// The valley radius as a fraction of the tip radius, in percent.
///
/// Thirty-eight is the pentagram — the star you get by joining every second
/// vertex of a regular pentagon — and it is the proportion a five-pointed star
/// is expected to have. Fatter looks like a flower, thinner like a splat.
const VALLEY_PERCENT: i32 = 38;

/// Gap between stars, as a fraction of the star's box.
const GAP_DIVISOR: i32 = 8;

/// A row of stars filled to a value, as an input or as a read-out.
///
/// ```
/// # use denise_ui::widgets::Rating;
/// # enum Msg { Rated(f32) }
/// Rating::new(3.0, Msg::Rated);         // interactive, five stars
/// Rating::<Msg>::display(4.3).with_max(5); // an average, read-only, so the
///                                        // message type has nothing to infer from
/// ```
///
/// # The value is continuous and the gesture is not
///
/// The value is an `f32` clamped to `0..=max`, so an average of `4.3` draws
/// four stars and a bit — which is the whole reason a rating is not simply a
/// count. Input, though, snaps to whole stars: a person taps the fourth star
/// and means four. That is exactly [`Slider`](super::Slider)'s split between a
/// continuous value and a quantised gesture, and it is why one widget covers
/// both the survey and the summary.
///
/// The value contract is [`Progress`](super::Progress)'s otherwise: NaN is
/// zero, out of range clamps, and [`update`](Rating::update) reports whether
/// the value actually moved — because a panel writes its readings every cycle
/// and repainting for an unchanged one is how an idle device stops being idle.
///
/// # One node, so one tab stop
///
/// Left and Right adjust by a star, Home and End go to the ends. The same rule
/// [`RadioGroup`](super::RadioGroup) and [`Tabs`](super::Tabs) follow, and for
/// the same reason: five stars that were five tab stops would be five things
/// to escape from.
///
/// # Getting back to zero
///
/// Nothing about tapping stars can reach zero — there is no zeroth star to
/// tap. Home does it, but a touch panel has no Home key, so
/// [`clearable`](Rating::clearable) makes tapping the current value clear it.
/// Opt-in, because a tap that undoes itself surprises people who did not ask
/// for it, and plenty of ratings are never meant to return to unrated.
#[derive(Clone, Debug)]
pub struct Rating<M> {
    value: f32,
    max: u32,
    role: Role,
    message: Option<fn(f32) -> M>,
    clearable: bool,
}

impl<M> Rating<M> {
    /// An interactive rating of five stars, emitting the new value.
    pub fn new(value: f32, message: fn(f32) -> M) -> Self {
        Self {
            value: clamp(value, 5),
            max: 5,
            role: Role::Warning,
            message: Some(message),
            clearable: false,
        }
    }

    /// A read-only rating: not focusable, not hittable, no message.
    ///
    /// Most ratings on a panel are this — a number someone else chose, shown.
    pub fn display(value: f32) -> Self {
        Self {
            value: clamp(value, 5),
            max: 5,
            role: Role::Warning,
            message: None,
            clearable: false,
        }
    }

    /// Sets how many stars there are. Clamped to at least one, and the value
    /// comes with it.
    pub fn with_max(mut self, max: u32) -> Self {
        self.max = max.max(1);
        self.value = clamp(self.value, self.max);
        self
    }

    /// Sets the colour of the filled stars.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Lets a press on the current value clear it to zero.
    ///
    /// The only route to zero without a keyboard — see the type documentation.
    pub fn clearable(mut self) -> Self {
        self.clearable = true;
        self
    }

    /// The current value, always in `0.0..=max`.
    #[inline]
    pub const fn value(&self) -> f32 {
        self.value
    }

    /// How many stars there are.
    #[inline]
    pub const fn max(&self) -> u32 {
        self.max
    }

    /// Sets the value, clamped, with NaN as zero. Silent, like every setter
    /// here: the message reports what a person did.
    pub fn set_value(&mut self, value: f32) {
        self.value = clamp(value, self.max);
    }

    /// Sets the value, reporting whether it actually changed.
    pub fn update(&mut self, value: f32) -> bool {
        let value = clamp(value, self.max);
        let changed = value != self.value;
        self.value = value;
        changed
    }

    /// The width this many stars want at `height`, gaps included.
    ///
    /// Offered, and never called by the tree — the application does its own
    /// arithmetic and passes a rectangle, which is the line between this and a
    /// layout engine.
    pub fn preferred_width(&self, height: i32) -> i32 {
        width_of(height.max(0), self.max as i32)
    }

    /// Emits and repaints if `value` differs from the one held.
    fn commit(&mut self, value: f32, ctx: &mut EventCtx<'_, M>) -> Handled {
        let value = clamp(value, self.max);
        if value == self.value {
            // Handled either way, matching `Slider`: the widget acted on the
            // event even when the value was already where the press asked for.
            // Not about swallowing — the tree delivers a press to the hit-test
            // winner and never falls it through — so the only thing this costs
            // is a repaint of a widget that did not change, and the only thing
            // it buys is that "handled" means what it says.
            return Handled::Yes;
        }
        self.value = value;
        if let Some(message) = self.message {
            ctx.emit(message(value));
        }
        Handled::Yes
    }

    /// The next whole star above the current value, and the one below it.
    ///
    /// Integer arithmetic, and not only for tidiness: `f32::floor` and
    /// `f32::ceil` live in `std` because they need `libm`, and `denise-ui`
    /// builds `no_std`. A cast truncates, which for a value that is never
    /// negative — the clamp guarantees it — *is* the floor.
    ///
    /// The asymmetry is real rather than an oversight. Stepping up from a whole
    /// 3 goes to 4 and stepping down goes to 2, but from a fractional 4.3 the
    /// step down is 4: an average is between stars, so the star below it is the
    /// one it has passed.
    fn step_up(&self) -> f32 {
        (self.value as i32 + 1) as f32
    }

    fn step_down(&self) -> f32 {
        let whole = self.value as i32;
        if self.value > whole as f32 {
            whole as f32
        } else {
            (whole - 1) as f32
        }
    }

    /// Which star `x` falls on, as a whole rating of `1..=max`, or `None`
    /// outside the row.
    fn star_at(&self, bounds: Rect, x: i32) -> Option<u32> {
        let (side, step) = geometry(bounds, self.max);
        if side <= 0 {
            return None;
        }
        let offset = x - bounds.x;
        if offset < 0 {
            return None;
        }
        // The gap after a star belongs to it, so a press between two stars
        // picks the one on its left rather than nothing at all.
        let index = (offset / step.max(1)).min(self.max as i32 - 1);
        Some(index as u32 + 1)
    }
}

impl<M> Default for Rating<M> {
    fn default() -> Self {
        Self::display(0.0)
    }
}

/// Into `0.0..=max`, with NaN as zero.
///
/// `f32::clamp` alone will not do: it propagates NaN rather than choosing an
/// end, so a `0.0 / 0.0` would reach the star geometry as a NaN fraction.
#[inline]
fn clamp(value: f32, max: u32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, max as f32)
    }
}

/// The width `max` stars of this side occupy, gaps included.
///
/// The one definition of the row's width. [`Rating::preferred_width`] is this
/// function and [`geometry`] inverts it, so the two cannot drift apart.
#[inline]
fn width_of(side: i32, max: i32) -> i32 {
    side * max + (side / GAP_DIVISOR) * (max - 1)
}

/// The side of one star's square box, and the distance between two boxes.
///
/// Stars are square and as tall as the row allows, so a rating in a short wide
/// rectangle is a row of stars rather than a row of ellipses.
///
/// The gap is a whole number of pixels, which makes this a genuine inverse of
/// [`width_of`] rather than a rearrangement of it: solving the proportion
/// directly is off by a pixel whenever `side / GAP_DIVISOR` truncates, and the
/// error grows with the star count. So the closed form is only a starting
/// guess, corrected by at most a step or two in each direction.
fn geometry(bounds: Rect, max: u32) -> (i32, i32) {
    let max = max.max(1) as i32;
    if bounds.width <= 0 || bounds.height <= 0 {
        return (0, 0);
    }
    let guess = (bounds.width * GAP_DIVISOR) / (max * (GAP_DIVISOR + 1) - 1).max(1);
    let mut side = guess.min(bounds.height).max(0);
    while side > 0 && width_of(side, max) > bounds.width {
        side -= 1;
    }
    while side < bounds.height && width_of(side + 1, max) <= bounds.width {
        side += 1;
    }
    (side, side + side / GAP_DIVISOR)
}

impl<M: 'static> Widget<M> for Rating<M> {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn measure(&self, _ctx: &mut MeasureCtx<'_>, offered: Offer) -> Measured {
        // Width for a height, the mirror of `Alert`: stars are square, so how
        // wide this wants to be is entirely a question about how tall it is.
        Measured {
            width: offered.height.map(|h| self.preferred_width(h)),
            height: None,
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        let bounds = ctx.bounds;
        let (side, step) = geometry(bounds, self.max);
        if side <= 0 {
            return;
        }
        let radius = side / 2;
        let valley = (radius * VALLEY_PERCENT / 100).max(1);
        // Centred vertically; the row starts at the left edge.
        let top = bounds.y + (bounds.height - side) / 2;

        let (empty, fill) = star_colors(ctx.theme, ctx.state, self.role);

        for i in 0..self.max as i32 {
            let x = bounds.x + i * step;
            let centre = Point::new(x + radius, top + radius);
            canvas.fill_star(centre, radius, valley, 5, 0, empty);

            // How much of *this* star the value fills, as a fraction.
            let filled = (self.value - i as f32).clamp(0.0, 1.0);
            if filled <= 0.0 {
                continue;
            }
            if filled >= 1.0 {
                canvas.fill_star(centre, radius, valley, 5, 0, fill);
                continue;
            }
            // A partial star is the same star drawn again through a narrower
            // clip. Rectangular clipping already does this exactly, so there is
            // no second shape to keep in step with the first — and the fraction
            // rounds outward by a pixel so that "just started" is visible, the
            // floor `Progress` and `RadialProgress` both keep.
            let width = ((side as f32 * filled) as i32 + 1).clamp(1, side);
            let mut c = canvas.with_clip(Rect::new(x, top, width, side));
            c.fill_star(centre, radius, valley, 5, 0, fill);
        }

        if ctx.state.contains(VisualState::FOCUSED) {
            focus_ring(
                ctx.theme,
                bounds,
                ctx.theme.radius(denise::Radius::Field),
                canvas,
            );
        }
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        if self.message.is_none() {
            return Handled::No;
        }
        let bounds = ctx.bounds;

        match event {
            Event::Input(InputEvent::PointerButton {
                state: ElementState::Down,
                position,
                ..
            })
            | Event::Input(InputEvent::TouchDown { position, .. }) => {
                if !bounds.contains(*position) {
                    return Handled::No;
                }
                let Some(star) = self.star_at(bounds, position.x) else {
                    return Handled::No;
                };
                ctx.request_focus();
                let target = star as f32;
                // Pressing the star the value already sits on clears it, when
                // asked for: the only way to zero without a keyboard.
                if self.clearable && self.value == target {
                    return self.commit(0.0, ctx);
                }
                self.commit(target, ctx)
            }

            Event::Input(InputEvent::Key {
                code,
                state: ElementState::Down,
                ..
            }) if ctx.state.contains(VisualState::FOCUSED) => {
                let value = match code {
                    // Down joins Left so the arrow cluster works whichever way
                    // a person reaches for "less", as `Slider` does.
                    KeyCode::ArrowLeft | KeyCode::ArrowDown => self.step_down(),
                    KeyCode::ArrowRight | KeyCode::ArrowUp => self.step_up(),
                    KeyCode::Home => 0.0,
                    KeyCode::End => self.max as f32,
                    _ => return Handled::No,
                };
                self.commit(value, ctx)
            }

            _ => Handled::No,
        }
    }

    fn accepts_pointer(&self) -> bool {
        self.message.is_some()
    }

    fn focusable(&self) -> bool {
        self.message.is_some()
    }
}

/// The empty star's colour and the filled one's.
///
/// Its own function for the reason [`ring_colors`](super::radial::ring_colors)
/// is: `interactive_pair` recesses **every** role to `Base200` when disabled,
/// so a disabled rating drawn straight from it loses information.
///
/// A rating loses *more* than a ring does, and that is what makes this the
/// fifth outing for the trap rather than a repeat of the fourth. A ring's track
/// is decoration; a rating's empty stars are the **denominator**. Recessing
/// them to `Base200` puts them within a shade of the panel, and a disabled
/// "two of five" reads as a plain "two" — which is not a muted control, it is a
/// wrong one. Found by rendering the showcase and looking at it, like the four
/// before it.
///
/// So a disabled rating recesses only its *fill*, and the empty stars keep the
/// `Base300` they always had.
pub(crate) fn star_colors(
    theme: &denise::Theme,
    state: VisualState,
    role: Role,
) -> (denise::Color, denise::Color) {
    if state.contains(VisualState::DISABLED) {
        // The track stays where it is, and the value steps up towards the
        // panel's own text colour — as far as it can afford, which is what
        // `muted` works out. Plainly disabled, still plainly five stars.
        let empty = theme.color(Role::Base300);
        let fill = crate::widgets::style::muted(empty, theme.color(Role::BaseContent));
        (empty, fill)
    } else {
        let (empty, _) = interactive_pair(theme, Role::Base300, state);
        (empty, interactive_pair(theme, role, state).0)
    }
}

impl<M> Describe for Rating<M> {
    const KIND: &'static str = "rating";
    const DOC: &'static str = "Stars, filled to a value and set by pressing one.";
    const GROUP: Group = Group::Input;
    const ICON: &'static denise::icon::Icon = &super::icons::RATING;

    const PROPERTIES: &'static [Property] = &[
        Property::new(
            "value",
            PropertyKind::Float { min: 0.0, max: 5.0 },
            "How many stars are filled; fractional, so an average of `4.3` draws four stars and a bit.",
        ),
        Property::new(
            "max",
            PropertyKind::Int { min: 1, max: 10 },
            "How many symbols there are.",
        ),
        Property::new(
            "on-change",
            PropertyKind::Message(Payload::Number),
            "Emitted with the new value when a person rates. Omitted, the rating is display-only.",
        ),
        Property::new(
            "clearable",
            PropertyKind::Bool,
            "Whether pressing the current value clears it to zero — the only route to zero without a keyboard.",
        ),
        Property::new(
            "role",
            PropertyKind::Enum(ROLES),
            "Colour of the filled stars.",
        ),
    ];

    fn get(&self, name: &str) -> Option<Value> {
        Some(match name {
            "value" => Value::Float(self.value),
            "max" => Value::Int(self.max as i32),
            "clearable" => Value::Bool(self.clearable),
            "role" => Value::role(self.role),
            // The message is the application's own type; see the `describe`
            // module documentation.
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            // The editor range above is for five stars, the default; the value
            // is really clamped against `max`, which `set_value` is where that
            // happens.
            "value" => self.set_value(value.as_float()?),
            "max" => {
                // `with_max`'s rule, and the value comes with it: a four left
                // over on a row shrunk to three stars would point past the end.
                self.max = value.as_count()?.max(1);
                self.set_value(self.value);
            }
            "clearable" => self.clearable = value.as_bool()?,
            "role" => self.role = value.as_role()?,
            "on-change" => return Err(Mismatch::Supplied),
            _ => return Err(Mismatch::Unknown),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    enum Msg {
        Rated(f32),
    }

    fn rating() -> Rating<Msg> {
        Rating::new(0.0, Msg::Rated)
    }

    #[test]
    fn the_value_clamps_to_the_star_count() {
        assert_eq!(Rating::<Msg>::display(9.0).value(), 5.0);
        assert_eq!(Rating::<Msg>::display(-9.0).value(), 0.0);
        assert_eq!(Rating::<Msg>::display(3.0).with_max(2).value(), 2.0);
        assert_eq!(rating().with_max(0).max(), 1, "zero stars is one star");
    }

    /// `done / total` with a zero total reaches widgets that never asked for it.
    #[test]
    fn a_value_that_is_not_a_number_is_zero_stars() {
        let zero_over_zero = core::hint::black_box(0.0f32) / core::hint::black_box(0.0f32);
        assert!(zero_over_zero.is_nan(), "the premise");
        assert_eq!(clamp(zero_over_zero, 5), 0.0);

        let mut r = rating();
        r.set_value(3.0);
        r.set_value(zero_over_zero);
        assert_eq!(r.value(), 0.0, "and it does not keep the old value");
    }

    #[test]
    fn infinities_clamp_to_the_end_they_point_at() {
        assert_eq!(clamp(f32::INFINITY, 5), 5.0);
        assert_eq!(clamp(f32::NEG_INFINITY, 5), 0.0);
    }

    /// Stars are square and centred, and the row never escapes its rectangle
    /// however the caller shapes it.
    #[test]
    fn the_row_fits_inside_any_rectangle() {
        for bounds in [
            Rect::new(0, 0, 200, 40),
            Rect::new(10, 10, 40, 200),
            Rect::new(0, 0, 3, 3),
            Rect::new(0, 0, 1, 100),
        ] {
            for max in [1u32, 3, 5, 10] {
                let (side, step) = geometry(bounds, max);
                assert!(
                    side <= bounds.height,
                    "{bounds:?} {max}: taller than its box"
                );
                if side == 0 {
                    continue;
                }
                let used = step * (max as i32 - 1) + side;
                assert!(
                    used <= bounds.width,
                    "{bounds:?} {max}: {used} wide in {}",
                    bounds.width
                );
            }
        }
    }

    /// The width a row asks for is a width the row then fits in — otherwise the
    /// query is worse than useless, because a caller that trusts it gets a
    /// squashed control.
    #[test]
    fn the_preferred_width_is_wide_enough_for_the_stars_it_asked_for() {
        for height in [8, 16, 24, 40, 100] {
            for max in [1u32, 3, 5, 10] {
                let r = Rating::<Msg>::display(0.0).with_max(max);
                let width = r.preferred_width(height);
                let bounds = Rect::new(0, 0, width, height);
                let (side, _) = geometry(bounds, max);
                assert_eq!(
                    side, height,
                    "height {height} max {max}: asked {width}, got stars of {side}"
                );
            }
        }
    }

    /// A press anywhere on the row picks a star, and the gap after a star
    /// belongs to it — a press that landed between two stars must not be
    /// silently dropped.
    #[test]
    fn every_x_across_the_row_picks_a_star() {
        let bounds = Rect::new(20, 10, 200, 40);
        let r = rating();
        let mut seen = [false; 5];
        for x in bounds.x..bounds.right() {
            let star = r.star_at(bounds, x).expect("inside the row");
            assert!((1..=5).contains(&star), "x={x} gave star {star}");
            seen[star as usize - 1] = true;
        }
        assert!(
            seen.iter().all(|&s| s),
            "some star was unreachable: {seen:?}"
        );
    }

    /// Presses walk left to right: star numbers never go backwards as x grows.
    #[test]
    fn the_star_under_the_pointer_never_goes_backwards() {
        let bounds = Rect::new(0, 0, 173, 33);
        let r = rating();
        let mut previous = 0;
        for x in bounds.x..bounds.right() {
            let star = r.star_at(bounds, x).expect("inside");
            assert!(
                star >= previous,
                "x={x} went back to {star} from {previous}"
            );
            previous = star;
        }
        assert_eq!(previous, 5, "the last star was never reached");
    }

    /// The integer stepping that replaced `f32::floor` and `f32::ceil`, which
    /// are `std`-only and so unavailable to a `no_std` crate. Truncation is the
    /// floor here because the value is never negative.
    #[test]
    fn stepping_lands_on_whole_stars_from_anywhere() {
        let at = |v: f32| {
            let mut r = Rating::<Msg>::display(0.0);
            r.set_value(v);
            (r.step_down(), r.step_up())
        };
        assert_eq!(
            at(4.3),
            (4.0, 5.0),
            "an average steps to the stars either side"
        );
        assert_eq!(at(3.0), (2.0, 4.0), "a whole value steps past itself");
        assert_eq!(at(0.0), (-1.0, 1.0), "and the clamp catches the low end");
        assert_eq!(at(0.4), (0.0, 1.0));
        assert_eq!(at(5.0), (4.0, 6.0), "the clamp catches the high end too");
    }

    /// A press between two stars must land on one of them, never on nothing —
    /// the gap after a star belongs to it.
    #[test]
    fn a_press_in_the_gap_between_stars_still_picks_one() {
        let bounds = Rect::new(0, 0, 225, 40);
        let r = rating();
        let (side, step) = geometry(bounds, 5);
        assert!(step > side, "the premise: there is a gap to press in");
        for i in 0..4 {
            for x in (bounds.x + i * step + side)..(bounds.x + (i + 1) * step) {
                assert_eq!(
                    r.star_at(bounds, x),
                    Some(i as u32 + 1),
                    "x={x} in the gap after star {}",
                    i + 1
                );
            }
        }
    }

    #[test]
    fn writing_the_same_value_reports_no_change() {
        let mut r = rating();
        assert!(r.update(3.0));
        assert!(!r.update(3.0));
        assert!(r.update(4.0));
    }

    /// A read-only rating is invisible to Tab and to the pointer, so it can sit
    /// inside a row that is itself a button.
    #[test]
    fn a_display_rating_takes_no_input() {
        let r = Rating::<Msg>::display(3.0);
        assert!(!Widget::<Msg>::focusable(&r));
        assert!(!Widget::<Msg>::accepts_pointer(&r));

        let live = rating();
        assert!(Widget::<Msg>::focusable(&live));
        assert!(Widget::<Msg>::accepts_pointer(&live));
    }

    /// The theme's own tightest intentional surface step: `Base300` against
    /// `Base200`, which is a recessed shape on a panel and is exactly what an
    /// empty star is.
    ///
    /// A fixed number cannot do this job. `AA_LARGE` is WCAG's floor for
    /// *text*, and no pair of base surfaces in any built-in theme comes within
    /// half of it — holding stars to 3:1 holds them to a bar the theme's own
    /// vocabulary cannot clear. Nor does any single ratio separate the good
    /// cases from the bad: `Base300` on `Base200` is 1.18 in the light theme
    /// and reads fine, while `Base200` on `Base100` is 1.23 in the dark one and
    /// was the bug.
    ///
    /// What was actually wrong is sharper than a ratio. `Panel` fills with
    /// `Base200`, and the disabled empty star recessed to `Base200` — the
    /// **same colour as the panel it sat on**, a contrast of exactly 1.00. So
    /// the floor asks the theme what it considers a visible step and demands at
    /// least that, which adapts to a theme nobody has written yet.
    fn visible_step(theme: &denise::Theme) -> u32 {
        denise::theme::contrast_x100(theme.color(Role::Base200), theme.color(Role::Base300))
    }

    /// The empty stars are the denominator, so they have to be visible against
    /// the panel they sit on — in every theme and *every state*. Guarding only
    /// filled-against-empty is what let a disabled "two of five" render as a
    /// plain "two"; this is the assertion that was missing.
    #[test]
    fn the_empty_stars_are_visible_against_the_surfaces_they_sit_on() {
        use denise::Theme;
        use denise::theme::contrast_x100;

        for theme in Theme::BUILT_IN {
            let floor = visible_step(&theme);
            for state in [VisualState::NONE, VisualState::DISABLED] {
                let (empty, _) = star_colors(&theme, state, Role::Warning);
                for behind in [Role::Base100, Role::Base200] {
                    let ratio = contrast_x100(theme.color(behind), empty);
                    assert!(
                        ratio >= floor,
                        "{} {state:?}: empty stars on {behind:?} are {ratio}, floor is {floor}",
                        theme.name
                    );
                }
            }
        }
    }

    /// The fifth time this trap has been walked into, and the first time it was
    /// known about in advance — which did not stop it, because it arrived
    /// wearing a different face. See [`star_colors`].
    #[test]
    fn a_disabled_rating_still_shows_its_value() {
        use denise::Theme;
        use denise::theme::contrast_x100;

        for theme in Theme::BUILT_IN {
            for role in [Role::Warning, Role::Primary, Role::Error] {
                for state in [VisualState::DISABLED, VisualState::NONE] {
                    let (empty, fill) = star_colors(&theme, state, role);
                    let ratio = contrast_x100(empty, fill);
                    let floor = visible_step(&theme);
                    // Not merely a different word — a *visibly* different one.
                    // Inequality alone would pass on two greys nobody can tell
                    // apart, which is the bug in all but name.
                    assert!(
                        ratio >= floor,
                        "{} {role:?} {state:?}: filled against empty is {ratio}, floor is {floor}",
                        theme.name
                    );
                }
            }
        }
    }
}
