//! A track and a fill. The one widget here that is purely an output.

use denise::{Rect, Role};
use denise::Pen;

use crate::widget::{PaintCtx, Widget};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Property, PropertyKind, ROLES, Value,
};
use crate::widgets::style::interactive_pair;

/// A determinate progress bar, `0.0` to `1.0`.
///
/// Not interactive, not focusable, not a tab stop. It reports; it does not take
/// input.
///
/// The bar fills the rectangle it is given, rather than centring a fixed
/// thickness inside it the way [`Checkbox`](super::Checkbox) and
/// [`Toggle`](super::Toggle) do. There is no theme metric for a bar's thickness,
/// and inventing one would mean a caller who wants a chunky bar has to fight it.
/// Give it the rectangle you want filled.
///
/// # There is no text on it
///
/// A percentage drawn inside the bar sits on the fill at one end and on the track
/// at the other, so it needs two colours in one string to stay readable. That is
/// a real amount of machinery for a label, and the alternative is already good: a
/// [`Label`](super::Label) beside the bar, updated from the same number.
///
/// # There is no indeterminate mode
///
/// It used to be impossible — an indeterminate bar animates forever, and
/// `Ui::tick` once animated only the focused widget, which a progress bar never
/// is. [#19] removed that limitation, so this is now a choice rather than a
/// wall: an unbounded animation costs a wake per frame for as long as the node
/// is visible, and the widget that already spends it is
/// [`Spinner`](super::Spinner). A second way to say *something is happening*,
/// in a widget whose whole job is saying *how much has happened*, has not
/// earned its place.
///
/// [#19]: https://github.com/bisand/denise/issues/19
#[derive(Clone, Copy, Debug)]
pub struct Progress {
    value: f32,
    role: Role,
}

impl Progress {
    /// A bar at `value`, which is clamped — see [`Progress::set_value`].
    pub fn new(value: f32) -> Self {
        Self {
            value: clamp(value),
            role: Role::Primary,
        }
    }

    /// Sets the colour of the filled portion.
    ///
    /// `Warning` or `Error` for a bar that means something is running out rather
    /// than something is being achieved.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// The current value, always in `0.0..=1.0`.
    #[inline]
    pub const fn value(&self) -> f32 {
        self.value
    }

    /// Sets the value, clamped into range.
    ///
    /// **Clamped rather than asserted, and NaN is zero.** The number a caller
    /// passes is nearly always `done / total`, and `total` is eventually zero —
    /// on the first frame, on an empty queue, on a job that was cancelled before
    /// it was measured. A debug assertion would fire on a developer's machine and
    /// a release build would draw a bar of undefined width; a panic would take
    /// down a paint loop, and a panic inside a paint loop on a kiosk is a black
    /// screen with no way to report itself.
    ///
    /// So: NaN draws an empty bar, negative draws an empty bar, and anything
    /// above one draws a full one.
    pub fn set_value(&mut self, value: f32) {
        self.value = clamp(value);
    }

    /// Sets the value, reporting whether it actually changed.
    ///
    /// The [`Label::update`](super::Label::update) pattern, and for the same
    /// reason: a panel writes its readings every cycle whether or not they moved,
    /// and repainting for a value that did not change is how an idle device stops
    /// being idle.
    ///
    /// Note what this compares. Two values a hundredth apart are *different* and
    /// will both report `true`, while on a bar 80 pixels wide they are the same
    /// drawing. A caller updating far faster than its bar is wide should quantise
    /// before calling — the widget cannot do it, because it does not know how wide
    /// it is until it paints.
    pub fn update(&mut self, value: f32) -> bool {
        let value = clamp(value);
        let changed = value != self.value;
        self.value = value;
        changed
    }

    /// Replaces the colour role.
    pub fn set_role(&mut self, role: Role) {
        self.role = role;
    }
}

impl Default for Progress {
    fn default() -> Self {
        Self::new(0.0)
    }
}

/// Into `0.0..=1.0`, with NaN as zero.
///
/// `f32::clamp` alone will not do: it propagates NaN, so a `0.0 / 0.0` would
/// arrive at the rasteriser as a width of NaN and cast to an unspecified `i32`.
#[inline]
fn clamp(value: f32) -> f32 {
    // NaN first and `f32::clamp` for the rest, because `clamp` alone will not do
    // it: it *propagates* NaN rather than choosing an end, so a `0.0 / 0.0` would
    // reach the rasteriser as a width of NaN.
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

/// Filled pixels for a track `width` across at `value`.
///
/// Truncated rather than rounded, with one exception: any value above zero shows
/// at least one pixel. A job that has started and shows nothing looks like a job
/// that has not started, and on a bar of any useful width one pixel is a
/// rounding error against being wrong about whether anything is happening.
fn fill_width(width: i32, value: f32) -> i32 {
    // The `is_nan` is not redundant, and leaving it out is a real bug that looks
    // like tidy code: **every** comparison with NaN is false, so `value <= 0.0`
    // lets a NaN through, `value >= 1.0` lets it through again, `NaN as i32`
    // saturates to zero, and the one-pixel floor below turns that into a bar
    // claiming a job has started.
    //
    // `Progress` only ever calls this with an already-clamped value, so this is
    // belt and braces — but it is a free function, and a guard that relies on its
    // only caller staying careful is not a guard.
    if width <= 0 || value.is_nan() || value <= 0.0 {
        return 0;
    }
    if value >= 1.0 {
        return width;
    }
    let filled = (width as f32 * value) as i32;
    filled.clamp(1, width)
}

impl<M: 'static> Widget<M> for Progress {
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
        // A stadium, computed from the height rather than taken from
        // `Radius::Selector`: the theme's token is a fixed number of pixels, and a
        // bar six pixels tall with an eight-pixel corner radius is not a bar.
        // Capped by the width too, so a bar narrower than it is tall does not ask
        // for a radius wider than the rectangle it is rounding.
        let radius = (bounds.height / 2).min(bounds.width / 2);

        let (track, _) = interactive_pair(ctx.theme, Role::Base300, ctx.state);
        canvas.fill_rounded_rect(bounds, radius, track);

        let filled = fill_width(bounds.width, self.value);
        if filled == 0 {
            return;
        }
        let (fill, _) = interactive_pair(ctx.theme, self.role, ctx.state);
        let bar = Rect::new(bounds.x, bounds.y, filled, bounds.height);
        canvas.fill_rounded_rect(bar, radius.min(filled / 2), fill);
    }
}

impl Describe for Progress {
    const KIND: &'static str = "progress";
    const DOC: &'static str = "A bar that fills to show how far along something is.";
    const GROUP: Group = Group::Indicator;
    const ICON: &'static denise::icon::Icon = &super::icons::PROGRESS;

    const PROPERTIES: &'static [Property] = &[
        Property::new(
            "value",
            PropertyKind::Float { min: 0.0, max: 1.0 },
            "How much of the bar is filled, from empty at `0.0` to full at `1.0`.",
        ),
        Property::new(
            "role",
            PropertyKind::Enum(ROLES),
            "Colour of the filled portion; `warning` or `error` for a bar that means something is running out rather than something is being achieved.",
        ),
    ];

    fn get(&self, name: &str) -> Option<Value> {
        Some(match name {
            "value" => Value::Float(self.value),
            "role" => Value::role(self.role),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            // Through the setter rather than the field: the range above is what
            // an inspector should offer, and `set_value` is the one place that
            // decides what a number outside it — or a NaN — means.
            "value" => self.set_value(value.as_float()?),
            "role" => self.role = value.as_role()?,
            _ => return Err(Mismatch::Unknown),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The number a caller passes is nearly always `done / total`, and `total` is
    /// eventually zero. This is the test that says what happens then.
    #[test]
    fn a_value_that_is_not_a_number_draws_an_empty_bar() {
        // Through `black_box` so this stays the division a caller actually
        // writes rather than a folded `f32::NAN` constant — the point is that
        // `done / total` produces this, not that NaN exists.
        let done = core::hint::black_box(0.0f32);
        let total = core::hint::black_box(0.0f32);
        let zero_over_zero = done / total;
        assert!(zero_over_zero.is_nan(), "the premise");
        assert_eq!(clamp(zero_over_zero), 0.0);
        assert_eq!(fill_width(200, zero_over_zero), 0);

        let mut bar = Progress::new(0.5);
        bar.set_value(zero_over_zero);
        assert_eq!(
            bar.value(),
            0.0,
            "and it does not keep the old value either"
        );
    }

    /// Infinity is a direction rather than a mistake, so it clamps to the end it
    /// points at rather than to zero.
    #[test]
    fn infinities_clamp_to_the_end_they_point_at() {
        assert_eq!(clamp(f32::INFINITY), 1.0);
        assert_eq!(clamp(f32::NEG_INFINITY), 0.0);
        assert_eq!(fill_width(200, f32::INFINITY), 200);
        assert_eq!(fill_width(200, f32::NEG_INFINITY), 0);
    }

    /// Out of range is clamped, not asserted. A panic inside a paint loop on a
    /// kiosk is a black screen with no way to report itself.
    #[test]
    fn values_outside_the_range_are_clamped_rather_than_refused() {
        assert_eq!(clamp(-0.5), 0.0);
        assert_eq!(clamp(1.5), 1.0);
        assert_eq!(clamp(1e30), 1.0);
        assert_eq!(Progress::new(42.0).value(), 1.0);
        assert_eq!(Progress::new(-42.0).value(), 0.0);
    }

    /// The ends are exact. Half-full landing on 99 of 200 pixels would be a bar
    /// that never quite agrees with the number beside it.
    #[test]
    fn the_fill_is_exact_at_both_ends_and_in_the_middle() {
        assert_eq!(fill_width(200, 0.0), 0);
        assert_eq!(fill_width(200, 1.0), 200);
        assert_eq!(fill_width(200, 0.5), 100);
        assert_eq!(fill_width(200, 0.25), 50);
        assert_eq!(
            fill_width(101, 1.0),
            101,
            "an odd width still fills exactly"
        );
    }

    /// Zero and "barely started" have to look different, or a job that has begun
    /// is indistinguishable from one that has not.
    #[test]
    fn a_value_just_above_zero_shows_something() {
        assert_eq!(fill_width(200, 0.0), 0);
        assert!(fill_width(200, 0.0001) >= 1);
        assert!(fill_width(2000, 1.0 / 5000.0) >= 1);
    }

    /// The fill never escapes the track, at any width including degenerate ones.
    #[test]
    fn the_fill_never_exceeds_the_track_at_any_width() {
        for width in [0, 1, 2, 3, 7, 200, 1920] {
            for step in 0..=20 {
                let value = step as f32 / 20.0;
                let filled = fill_width(width, value);
                assert!(
                    (0..=width.max(0)).contains(&filled),
                    "width {width} at {value} gave {filled}"
                );
            }
            assert_eq!(fill_width(width, 2.0), width.max(0), "width {width} full");
        }
        assert_eq!(fill_width(-5, 0.5), 0, "a negative width fills nothing");
    }

    /// Monotonic: more progress is never fewer pixels.
    #[test]
    fn more_progress_is_never_fewer_pixels() {
        for width in [1, 7, 200, 1920] {
            let mut previous = 0;
            for step in 0..=1000 {
                let filled = fill_width(width, step as f32 / 1000.0);
                assert!(
                    filled >= previous,
                    "width {width} went backwards at step {step}"
                );
                previous = filled;
            }
        }
    }

    /// A panel writes its readings every cycle whether or not they moved.
    /// Repainting for a value that did not change is how an idle device stops
    /// being idle.
    #[test]
    fn writing_the_same_value_reports_no_change() {
        let mut bar = Progress::new(0.0);
        assert!(bar.update(0.4));
        assert!(!bar.update(0.4));
        assert!(bar.update(0.6));

        // And through the clamp: two different out-of-range numbers are the same
        // value once clamped, so the second is not a change.
        assert!(bar.update(5.0));
        assert!(!bar.update(9.0));
        assert!(!bar.update(f32::INFINITY));
    }
}
