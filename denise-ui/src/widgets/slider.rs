//! A value in a range, dragged or typed.

use denise::{ElementState, InputEvent, KeyCode, Point, Rect, Role, Theme};
use denise_render::Canvas;

use crate::widget::{Event, EventCtx, Handled, PaintCtx, VisualState, Widget};
use crate::widgets::style::{focus_ring, interactive_pair};

/// A horizontal slider over `min..=max`.
///
/// The message carries the value itself rather than a fraction, so a setpoint
/// reads as one:
///
/// ```
/// # use denise_ui::Slider;
/// enum Message { Setpoint(f32) }
/// Slider::new(16.0, 30.0, 21.5, Message::Setpoint).with_step(0.5);
/// ```
///
/// # Dragging keeps the pointer even when it leaves
///
/// This is the whole reason a slider belongs in a toolkit rather than in every
/// application. The tree already routes moves to the pressed widget wherever the
/// pointer goes — but it **clears [`VisualState::PRESSED`] the moment the pointer
/// leaves the widget**, because that is what makes a button's drag-off cancel.
/// A slider that read its drag state from `PRESSED` would therefore stop tracking
/// at exactly the edge it most needs to keep tracking past.
///
/// So the drag is a flag of this widget's own, set on press and cleared on
/// release, and the pressed *look* comes from that flag rather than from the
/// tree's. A drag that runs off either end clamps and keeps following, and comes
/// back when the pointer does.
///
/// The one thing that flag assumes is that every press is eventually followed by
/// a release. That is the input stream's contract, and every backend here honours
/// it; there is no event that means "your capture was taken away".
///
/// # Pressing the track jumps
///
/// Rather than stepping towards the press. A panel is touch-first, and a finger
/// landing on a point of the track means *go there* — there is no cursor
/// hovering to suggest anything else. It also makes the whole track a target
/// instead of just the knob, which matters when the knob is 20 pixels wide and
/// the finger is not.
#[derive(Clone, Debug)]
pub struct Slider<M> {
    min: f32,
    max: f32,
    value: f32,
    step: Option<f32>,
    dragging: bool,
    message: Option<fn(f32) -> M>,
    role: Role,
}

impl<M> Slider<M> {
    /// A slider over `min..=max`, starting at `value`.
    ///
    /// A reversed range is put the right way round rather than refused: it is a
    /// caller's argument order, not a state the widget has to model.
    pub fn new(min: f32, max: f32, value: f32, message: fn(f32) -> M) -> Self {
        let (min, max) = order(min, max);
        Self {
            min,
            max,
            value: clamp(min, max, value),
            step: None,
            dragging: false,
            message: Some(message),
            role: Role::Primary,
        }
    }

    /// A slider that emits nothing, for a value the application reads rather
    /// than reacts to.
    pub fn inert(min: f32, max: f32, value: f32) -> Self {
        let (min, max) = order(min, max);
        Self {
            min,
            max,
            value: clamp(min, max, value),
            step: None,
            dragging: false,
            message: None,
            role: Role::Primary,
        }
    }

    /// Snaps to multiples of `step` from `min`.
    ///
    /// Off by default, because a continuous slider is what a brightness or a
    /// volume wants. A step that is zero, negative or not a number is ignored
    /// rather than dividing by it.
    pub fn with_step(mut self, step: f32) -> Self {
        self.step = (step.is_finite() && step > 0.0).then_some(step);
        self.value = self.settle(self.value);
        self
    }

    /// Sets the colour role of the filled portion and the knob.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// The current value, always within the range and on a step if there is one.
    #[inline]
    pub const fn value(&self) -> f32 {
        self.value
    }

    /// The range, in order.
    #[inline]
    pub const fn range(&self) -> (f32, f32) {
        (self.min, self.max)
    }

    /// Sets the value **without emitting anything**, clamped and snapped.
    ///
    /// Silent for the same reason [`Checkbox::set_checked`] is: the message
    /// reports what a person did.
    ///
    /// [`Checkbox::set_checked`]: super::Checkbox::set_checked
    pub fn set_value(&mut self, value: f32) {
        self.value = self.settle(value);
    }

    /// Sets the value, reporting whether it actually changed.
    pub fn update(&mut self, value: f32) -> bool {
        let value = self.settle(value);
        let changed = value != self.value;
        self.value = value;
        changed
    }

    /// Replaces the range, keeping the value inside it.
    pub fn set_range(&mut self, min: f32, max: f32) {
        let (min, max) = order(min, max);
        self.min = min;
        self.max = max;
        self.value = self.settle(self.value);
    }

    /// Replaces the colour role.
    pub fn set_role(&mut self, role: Role) {
        self.role = role;
    }

    /// Whether a drag is in progress.
    #[inline]
    pub const fn dragging(&self) -> bool {
        self.dragging
    }

    /// `max - min`, never negative and never NaN.
    #[inline]
    fn span(&self) -> f32 {
        self.max - self.min
    }

    /// Clamped into the range and put on a step.
    fn settle(&self, value: f32) -> f32 {
        let value = clamp(self.min, self.max, value);
        let Some(step) = self.step else {
            return value;
        };
        // Rounded without `f32::round`, which lives in `std` — the same idiom
        // `Metrics::scaled` uses. `(value - min) / step` is never negative, so
        // adding a half and truncating is a round.
        let steps = ((value - self.min) / step + 0.5) as i32;
        clamp(self.min, self.max, self.min + steps as f32 * step)
    }

    /// How far along the range the value sits, `0.0..=1.0`.
    fn fraction(&self) -> f32 {
        let span = self.span();
        if span <= 0.0 {
            return 0.0;
        }
        ((self.value - self.min) / span).clamp(0.0, 1.0)
    }

    /// One arrow-key press.
    ///
    /// A hundredth of the range when there is no step, which puts a full sweep
    /// at a hundred presses — enough to be precise, few enough to be usable.
    fn small_step(&self) -> f32 {
        self.step.unwrap_or_else(|| self.span() / 100.0)
    }

    /// One `PageUp` or `PageDown`, ten times the small step.
    fn large_step(&self) -> f32 {
        self.small_step() * 10.0
    }

    /// Applies a new value, emitting only if it actually moved.
    fn commit(&mut self, value: f32, ctx: &mut EventCtx<'_, M>) -> Handled {
        let value = self.settle(value);
        if value != self.value {
            self.value = value;
            if let Some(message) = self.message {
                ctx.emit(message(value));
            }
        }
        // Handled either way: the widget acted on the event even when the value
        // was already where the press asked for.
        Handled::Yes
    }
}

/// The two ends, in order.
#[inline]
fn order(a: f32, b: f32) -> (f32, f32) {
    if b < a { (b, a) } else { (a, b) }
}

/// Into the range, with NaN going to the low end.
///
/// NaN rather than a panic for the reason [`Progress`](super::Progress) gives at
/// more length: the number comes from a caller's arithmetic, and a panic inside a
/// paint loop on a kiosk is a black screen.
#[inline]
fn clamp(min: f32, max: f32, value: f32) -> f32 {
    if value.is_nan() {
        min
    } else {
        value.clamp(min, max)
    }
}

/// The knob's diameter for a given rectangle and theme.
fn knob_size(bounds: Rect, theme: &Theme) -> i32 {
    theme
        .metrics
        .size_selector
        .min(bounds.height)
        .min(bounds.width)
        .max(1)
}

/// The leftmost and rightmost the knob's **centre** may sit.
///
/// Inset by a radius at each end, so the knob stays inside the rectangle at both
/// extremes rather than half-escaping it.
fn travel(bounds: Rect, diameter: i32) -> (i32, i32) {
    let radius = diameter / 2;
    let left = bounds.x + radius;
    let right = bounds.right() - diameter + radius;
    (left, right.max(left))
}

/// Where along the travel a pointer at `x` is asking for, `0.0..=1.0`.
fn fraction_at(bounds: Rect, diameter: i32, x: i32) -> f32 {
    let (left, right) = travel(bounds, diameter);
    if right <= left {
        return 0.0;
    }
    ((x - left) as f32 / (right - left) as f32).clamp(0.0, 1.0)
}

impl<M: 'static> Widget<M> for Slider<M> {
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        let bounds = ctx.bounds;
        if bounds.is_empty() {
            return;
        }
        let diameter = knob_size(bounds, ctx.theme);
        let (left, right) = travel(bounds, diameter);
        let centre = left + (((right - left) as f32) * self.fraction()) as i32;

        // The pressed look comes from this widget's own flag, not from the tree's
        // `PRESSED` — the tree clears that when the pointer leaves, and a knob
        // that stops looking held while it is still being dragged is a lie about
        // what is going on.
        let state = ctx.state.set(VisualState::PRESSED, self.dragging);

        let thickness = (diameter / 4).max(2);
        let track = Rect::new(
            bounds.x,
            bounds.y + (bounds.height - thickness) / 2,
            bounds.width,
            thickness,
        );
        let radius = thickness / 2;

        let (unfilled, _) = interactive_pair(ctx.theme, Role::Base300, state);
        canvas.fill_rounded_rect(track, radius, unfilled);

        let (fill, rim) = interactive_pair(ctx.theme, self.role, state);
        let filled_to = centre - track.x;
        if filled_to > 0 {
            let filled = Rect::new(track.x, track.y, filled_to, track.height);
            canvas.fill_rounded_rect(filled, radius.min(filled_to / 2), fill);
        }

        let knob = Rect::new(
            centre - diameter / 2,
            bounds.y + (bounds.height - diameter) / 2,
            diameter,
            diameter,
        );
        canvas.fill_rounded_rect(knob, diameter / 2, fill);
        // A rim in the role's own content colour, which is the one pairing the
        // theme guarantees. Without it the knob's left half vanishes into the
        // filled track it is sitting on — same colour, no edge.
        canvas.stroke_rounded_rect(knob, diameter / 2, ctx.theme.metrics.border, rim);

        if state.contains(VisualState::FOCUSED) {
            focus_ring(
                ctx.theme,
                bounds,
                ctx.theme.radius(denise::Radius::Field),
                canvas,
            );
        }
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        // Everything the mapping needs, copied out: the closure must not borrow
        // `self` or `ctx`, because `commit` needs both mutably.
        let bounds = ctx.bounds;
        let diameter = knob_size(bounds, ctx.theme);
        let (min, span) = (self.min, self.span());
        let at = move |point: Point| min + fraction_at(bounds, diameter, point.x) * span;

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
                self.dragging = true;
                let value = at(*position);
                self.commit(value, ctx)
            }

            // Delivered wherever the pointer is, because the tree routes moves to
            // the pressed widget. Guarded by our own flag rather than by the
            // bounds: leaving the rectangle is the case this exists for.
            Event::Input(InputEvent::PointerMoved { position })
            | Event::Input(InputEvent::TouchMoved { position, .. }) => {
                if !self.dragging {
                    return Handled::No;
                }
                let value = at(*position);
                self.commit(value, ctx)
            }

            Event::Input(InputEvent::PointerButton {
                state: ElementState::Up,
                ..
            })
            | Event::Input(InputEvent::TouchUp { .. }) => {
                if !self.dragging {
                    return Handled::No;
                }
                self.dragging = false;
                Handled::Yes
            }

            Event::Input(InputEvent::Key {
                code,
                state: ElementState::Down,
                ..
            }) if ctx.state.contains(VisualState::FOCUSED) => {
                let value = match code {
                    KeyCode::ArrowLeft | KeyCode::ArrowDown => self.value - self.small_step(),
                    KeyCode::ArrowRight | KeyCode::ArrowUp => self.value + self.small_step(),
                    KeyCode::PageDown => self.value - self.large_step(),
                    KeyCode::PageUp => self.value + self.large_step(),
                    KeyCode::Home => self.min,
                    KeyCode::End => self.max,
                    _ => return Handled::No,
                };
                self.commit(value, ctx)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use denise::theme;

    fn slider() -> Slider<f32> {
        Slider::new(0.0, 100.0, 50.0, |value| value)
    }

    /// A caller's argument order is not a state the widget has to model.
    #[test]
    fn a_reversed_range_is_put_the_right_way_round() {
        let slider: Slider<f32> = Slider::new(30.0, 16.0, 21.0, |v| v);
        assert_eq!(slider.range(), (16.0, 30.0));
        assert_eq!(slider.value(), 21.0);
    }

    /// The degenerate range. Every fraction is zero and nothing divides by it.
    #[test]
    fn an_empty_range_pins_the_value_and_does_not_divide_by_zero() {
        let mut slider: Slider<f32> = Slider::new(5.0, 5.0, 5.0, |v| v);
        assert_eq!(slider.fraction(), 0.0);
        slider.set_value(99.0);
        assert_eq!(slider.value(), 5.0);
        assert!(slider.small_step().is_finite() || slider.small_step() == 0.0);
    }

    /// NaN goes to the low end rather than through the arithmetic.
    #[test]
    fn a_value_that_is_not_a_number_lands_at_the_low_end() {
        let done = core::hint::black_box(0.0f32);
        let total = core::hint::black_box(0.0f32);
        let mut slider = slider();
        slider.set_value(done / total);
        assert_eq!(slider.value(), 0.0);
        assert_eq!(slider.fraction(), 0.0);
    }

    #[test]
    fn values_outside_the_range_are_clamped() {
        let mut slider = slider();
        slider.set_value(1e9);
        assert_eq!(slider.value(), 100.0);
        slider.set_value(-1e9);
        assert_eq!(slider.value(), 0.0);
        slider.set_value(f32::NEG_INFINITY);
        assert_eq!(slider.value(), 0.0);
    }

    /// Snapping lands on multiples of the step measured **from `min`**, not from
    /// zero — a 16..30 slider stepping by 0.5 should offer 21.5, and it would
    /// offer nothing useful if the grid started somewhere else.
    #[test]
    fn a_step_snaps_to_multiples_from_the_low_end() {
        let mut slider: Slider<f32> = Slider::new(16.0, 30.0, 21.0, |v| v).with_step(0.5);
        slider.set_value(21.3);
        assert_eq!(slider.value(), 21.5);
        slider.set_value(21.2);
        assert_eq!(slider.value(), 21.0);

        // And never off the end, however the rounding falls.
        slider.set_value(29.9);
        assert!(slider.value() <= 30.0);
        slider.set_value(16.1);
        assert!(slider.value() >= 16.0);
    }

    /// A step that cannot divide anything is ignored rather than dividing by it.
    #[test]
    fn a_nonsense_step_is_ignored() {
        for step in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let slider: Slider<f32> = Slider::new(0.0, 10.0, 3.3, |v| v).with_step(step);
            assert_eq!(slider.value(), 3.3, "step {step} should have been ignored");
        }
    }

    /// The knob stays inside its rectangle at both ends of the travel, which is
    /// the whole of what the inset arithmetic has to get right.
    #[test]
    fn the_knob_stays_inside_the_rectangle_across_the_whole_travel() {
        for width in [1, 2, 21, 200, 1920] {
            let bounds = Rect::new(7, 3, width, 40);
            let diameter = knob_size(bounds, &theme::DARK);
            let (left, right) = travel(bounds, diameter);
            assert!(right >= left, "width {width}: travel is inverted");
            for centre in [left, (left + right) / 2, right] {
                let knob = Rect::new(centre - diameter / 2, bounds.y, diameter, diameter);
                assert!(
                    knob.x >= bounds.x && knob.right() <= bounds.right(),
                    "width {width}: knob at {centre} escaped {bounds:?}"
                );
            }
        }
    }

    /// A pointer dragged past either end clamps rather than wrapping. Wrapping a
    /// volume from full to silent because a finger slid too far is the failure
    /// this pins.
    #[test]
    fn a_pointer_past_either_end_clamps_rather_than_wrapping() {
        let bounds = Rect::new(10, 0, 200, 40);
        let diameter = knob_size(bounds, &theme::DARK);

        assert_eq!(fraction_at(bounds, diameter, -100_000), 0.0);
        assert_eq!(fraction_at(bounds, diameter, 100_000), 1.0);
        assert_eq!(fraction_at(bounds, diameter, bounds.x - 1), 0.0);
        assert_eq!(fraction_at(bounds, diameter, bounds.right() + 1), 1.0);

        // And it is monotonic in between, so a drag never jumps backwards.
        let mut previous = -1.0;
        for x in bounds.x..bounds.right() {
            let fraction = fraction_at(bounds, diameter, x);
            assert!(fraction >= previous, "went backwards at x {x}");
            previous = fraction;
        }
    }

    /// A rectangle too narrow to have any travel must still answer, rather than
    /// dividing by a zero-length range.
    #[test]
    fn a_rectangle_with_no_travel_answers_zero() {
        let bounds = Rect::new(0, 0, 4, 40);
        let diameter = knob_size(bounds, &theme::DARK);
        assert_eq!(fraction_at(bounds, diameter, 2), 0.0);
        assert_eq!(fraction_at(bounds, diameter, 1000), 0.0);
    }

    /// The step sizes are the keyboard contract: a hundred presses for a sweep,
    /// ten pages.
    #[test]
    fn the_keyboard_steps_are_a_hundredth_and_a_tenth_of_the_range() {
        let slider = slider();
        assert_eq!(slider.small_step(), 1.0);
        assert_eq!(slider.large_step(), 10.0);

        // With a step, arrows move by exactly one of them.
        let stepped: Slider<f32> = Slider::new(0.0, 10.0, 0.0, |v| v).with_step(0.25);
        assert_eq!(stepped.small_step(), 0.25);
        assert_eq!(stepped.large_step(), 2.5);
    }

    /// The knob's rim has to separate it from the filled track it sits on — same
    /// colour otherwise, and no edge. `interactive_pair` is what guarantees it.
    #[test]
    fn the_knob_rim_is_visible_against_the_knob_in_every_theme_and_state() {
        use denise::theme::{AA_LARGE, contrast_x100};

        for theme in Theme::BUILT_IN {
            for state in [
                VisualState::NONE,
                VisualState::HOVERED,
                VisualState::PRESSED,
                VisualState::DISABLED,
                VisualState::FOCUSED,
            ] {
                let (fill, rim) = interactive_pair(&theme, Role::Primary, state);
                let ratio = contrast_x100(fill, rim);
                assert!(
                    ratio >= AA_LARGE,
                    "{} {state:?}: rim against knob is {ratio}, floor is {AA_LARGE}",
                    theme.name
                );
            }
        }
    }
}
