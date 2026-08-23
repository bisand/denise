//! A rotating arc, for when there is nothing to report but that something is
//! happening.

use denise::Role;
use denise_render::{Canvas, TURN};

use crate::motion::{Motion, Wake};
use crate::widget::{Animation, PaintCtx, Widget};
use crate::widgets::describe::{
    Describe, DynDescribe, Mismatch, Property, PropertyKind, ROLES, Value,
};
use crate::widgets::radial::{ring, ring_colors, thickness_for};

/// How long one revolution takes by default.
const PERIOD_MS: u64 = 1_000;

/// The shortest revolution a spinner will accept.
///
/// A period below the sampling interval would turn more than a full circle
/// between frames, which is a spinner that looks stopped or, worse, looks like
/// it is going backwards. The tree's interval is not knowable here — it belongs
/// to [`Motion`] and can be changed after this widget is built — so the clamp
/// uses the default one, which is the fastest rate anybody is likely to set.
const MIN_PERIOD_MS: u64 = Motion::DEFAULT_INTERVAL_MS;

/// How much of the ring the moving arc covers.
///
/// Three quarters: enough gap to see it turning, enough arc to read as a ring
/// rather than as a fragment.
const SWEEP: i32 = TURN * 3 / 4;

/// An indeterminate activity indicator: an arc that goes round and round.
///
/// Not interactive, not focusable, not a tab stop, and it holds no value — a
/// spinner that could show progress would be
/// [`RadialProgress`](super::RadialProgress).
///
/// # It must be started, and it must be stopped
///
/// **This is the widget that can keep a device awake.** It is unbounded by
/// nature: [`animate`](Widget::animate) never answers [`Wake::Never`] while the
/// node is visible, which is exactly what
/// [`Ui::request_animation`](crate::Ui::request_animation) says it is allowed to
/// do and exactly what it costs.
///
/// So it does not start itself. A spinner receives no events, so it cannot ask
/// for frames from an event handler; the application asks, at the moment it
/// decides something is loading:
///
/// ```
/// # use denise::{Rect, Size, theme};
/// # use denise_ui::{Ui, widgets::Panel};
/// # #[derive(Clone, Debug)] enum Msg { Noop }
/// # fn demo() -> Option<()> {
/// # let mut ui: Ui<Msg> = Ui::new(Size::new(1920, 1080), theme::DARK);
/// # let root = ui.root();
/// # use denise_ui::Spinner;
/// let id = ui.add(root, Spinner::new(), Rect::new(100, 80, 48, 48))?;
/// ui.request_animation(id);
/// # Some(()) }
/// ```
///
/// That is not an awkwardness to paper over with a constructor that does it
/// invisibly. Keeping a CPU awake is a decision, and this puts it at the line
/// where somebody made it.
///
/// **Stopping is hiding.** `Ui::set_visible(id, false)` — or removing the node
/// — takes it out of the animating set, and
/// [`Ui::animating`](crate::Ui::animating) drops back to zero. A spinner left
/// visible on a screen nobody is looking at is a device that never idles, and
/// nothing in the toolkit will notice on your behalf.
///
/// # Shape
///
/// A faint full ring with a brighter arc turning inside it, inscribed in the
/// rectangle it is given like [`RadialProgress`](super::RadialProgress) and
/// sharing its geometry — the same centre, radius and thickness rules, so a
/// spinner and a ring of the same size are the same ring.
#[derive(Clone, Copy, Debug)]
pub struct Spinner {
    role: Role,
    thickness: Option<i32>,
    period_ms: u64,
    /// This spinner's own sampling interval, overriding the tree's.
    ///
    /// `None` — the usual case — means it turns at whatever rate
    /// [`Motion`](crate::Motion) says, along with everything else.
    frame_ms: Option<u64>,
    /// How far into the current revolution the arc is, in milliseconds.
    ///
    /// **Time accumulates, not angle.** Adding a per-frame angle would truncate
    /// once per frame and lose a little of every lap — at 20 fps and a one
    /// second period that is 16 units of 65536 a lap, which is invisible and
    /// still wrong. Accumulating the milliseconds and deriving the angle from
    /// them means a whole period is exactly a whole turn, forever.
    phase_ms: u64,
    /// The clock reading `angle` was computed at, or `None` before the first
    /// frame.
    last_ms: Option<u64>,
}

impl Spinner {
    /// A spinner in [`Role::Primary`], one revolution a second.
    pub fn new() -> Self {
        Self {
            role: Role::Primary,
            thickness: None,
            period_ms: PERIOD_MS,
            frame_ms: None,
            phase_ms: 0,
            last_ms: None,
        }
    }

    /// Sets the colour of the moving arc.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Sets the ring's thickness in pixels, instead of deriving it from the
    /// radius.
    pub fn with_thickness(mut self, thickness: i32) -> Self {
        self.thickness = Some(thickness);
        self
    }

    /// Sets how long one revolution takes.
    ///
    /// Clamped to the default sampling interval,
    /// [`Motion::DEFAULT_INTERVAL_MS`](crate::Motion::DEFAULT_INTERVAL_MS): a
    /// period shorter than a frame turns more than a full circle between them,
    /// which looks stopped or, worse, looks like it is going backwards.
    pub fn with_period_ms(mut self, period_ms: u64) -> Self {
        self.period_ms = period_ms.max(MIN_PERIOD_MS);
        self
    }

    /// Gives this spinner its own sampling interval, in milliseconds.
    ///
    /// **Almost nothing should call this.** The rate belongs to the tree —
    /// [`Ui::set_motion`](crate::Ui::set_motion) — so that one decision covers
    /// every moving thing on the panel and a deployment can turn all of it down
    /// at once. This is the escape hatch for the spinner that genuinely differs
    /// from everything around it: a ring that must keep turning smoothly on a
    /// panel whose other animation has been coarsened, or a decorative one that
    /// should cost less than the rest.
    ///
    /// It overrides [`Motion::Every`] and is overridden by
    /// [`Motion::None`](crate::Motion::None) in turn — reduced motion is a
    /// person's decision, and a widget does not get to opt out of it.
    pub fn with_frame_ms(mut self, frame_ms: u64) -> Self {
        self.frame_ms = Some(frame_ms.max(1));
        self
    }

    /// Where the arc currently starts, in [`TURN`] units.
    #[inline]
    pub fn angle(&self) -> i32 {
        angle_at(self.phase_ms, self.period_ms)
    }

    /// Replaces the colour role.
    pub fn set_role(&mut self, role: Role) {
        self.role = role;
    }

    /// Replaces the revolution period, clamped as [`Spinner::with_period_ms`].
    pub fn set_period_ms(&mut self, period_ms: u64) {
        self.period_ms = period_ms.max(MIN_PERIOD_MS);
    }

    /// Sets or clears this spinner's own sampling interval — see
    /// [`Spinner::with_frame_ms`], which is where the argument for not using it
    /// is written down.
    pub fn set_frame_ms(&mut self, frame_ms: Option<u64>) {
        self.frame_ms = frame_ms.map(|ms| ms.max(1));
    }
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}

/// The arc's start angle at `phase_ms` into a revolution of `period_ms`.
///
/// A pure function of the phase, which is what makes a whole period exactly a
/// whole turn: nothing is accumulated in [`TURN`] units, so nothing rounds
/// twice.
fn angle_at(phase_ms: u64, period_ms: u64) -> i32 {
    let period = period_ms.max(1);
    ((phase_ms % period) * TURN as u64 / period) as i32
}

impl<M: 'static> Widget<M> for Spinner {
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

        // Shared with `RadialProgress` so a spinner and a ring of the same size
        // are the same ring — including when disabled, where the arc has to
        // stay distinguishable from the track it sits on.
        let (track, arc) = ring_colors(ctx.theme, ctx.state, self.role);
        canvas.stroke_circle(centre, radius, thickness, track);
        canvas.stroke_arc(centre, radius, thickness, self.angle(), SWEEP, arc);
    }

    fn animate(&mut self, now_ms: u64) -> Animation {
        // The first frame establishes the epoch and moves nothing: without this
        // the spinner would jump by however long the application had been
        // running before somebody asked it to spin.
        let elapsed = match self.last_ms {
            Some(last) => now_ms.saturating_sub(last),
            None => 0,
        };
        self.last_ms = Some(now_ms);

        // Capped at one period, so a spinner hidden for an hour and shown again
        // resumes rather than winding an hour of rotation forward to land in the
        // same place.
        //
        // The modulo here bounds the *field*, and `angle_at` takes it again to
        // stay a total function of whatever it is handed. Either alone would
        // draw the same pixels — a mutation removing this one changes nothing
        // observable, which is how that was established — and both stay for the
        // reason `Progress::fill_width` keeps its own belt-and-braces guard: a
        // check that relies on its only caller staying careful is not a check.
        let before = self.angle();
        let period = self.period_ms.max(1);
        self.phase_ms = (self.phase_ms + elapsed.min(period)) % period;

        Animation {
            // A frame that moved nothing owes no repaint. The tree wakes for the
            // most impatient animation and asks everybody, so a spinner is
            // routinely asked before the time it wanted.
            repaint: self.angle() != before,
            // Never `Wake::Never`. This is the unbounded case #19 made
            // expressible, and the only thing that stops it is the node going
            // away — or motion being turned off, which is what [`Widget::snap`] is for.
            //
            // How fast "animating" is belongs to the tree. The override says a
            // time instead, saturating because the clock is the application's
            // and its value is not this widget's to assume anything about.
            next: match self.frame_ms {
                None => Wake::Animating,
                Some(ms) => Wake::At(now_ms.saturating_add(ms)),
            },
        }
    }

    /// Nothing to land: a spinner has no end state to arrive at, so under
    /// [`Motion::None`](crate::Motion::None) it simply stops turning and leaves
    /// a still ring. That is the honest reading of "no motion" for the one
    /// widget that is nothing but motion.
    fn snap(&mut self, _now_ms: u64) -> Animation {
        Animation::NONE
    }
}

impl Describe for Spinner {
    const KIND: &'static str = "spinner";

    const PROPERTIES: &'static [Property] = &[
        Property::new(
            "role",
            PropertyKind::Enum(ROLES),
            "Colour of the moving arc. The faint track behind it is derived from the same role.",
        ),
        Property::new(
            "thickness",
            PropertyKind::Int { min: 1, max: 64 },
            "Ring width in pixels. Derived from the node's size without it.",
        ),
        Property::new(
            "period-ms",
            // The floor is the clamp `set_period_ms` applies, named rather than
            // repeated, so an editor cannot offer a period the widget refuses.
            PropertyKind::Int {
                min: MIN_PERIOD_MS as i32,
                max: 10_000,
            },
            "How long one full turn takes, in milliseconds.",
        ),
        Property::new(
            "frame-ms",
            PropertyKind::Int { min: 1, max: 1_000 },
            "This spinner's own sampling interval, overriding the tree's — a coarse one gives a ticking rather than a sweeping hand. Almost nothing should set it.",
        ),
    ];

    fn get(&self, name: &str) -> Option<Value> {
        Some(match name {
            "role" => Value::role(self.role),
            // Both of these are derived when unset, and a derived value is not
            // one to write into a file as though somebody had chosen it.
            "thickness" => Value::Int(self.thickness?),
            "period-ms" => Value::Int(i32::try_from(self.period_ms).unwrap_or(i32::MAX)),
            "frame-ms" => Value::Int(i32::try_from(self.frame_ms?).unwrap_or(i32::MAX)),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            "role" => self.role = value.as_role()?,
            // A ring thinner than a pixel is a ring nobody can see.
            "thickness" => self.thickness = Some(value.as_int()?.max(1)),
            // Through the setters, which hold the two clamps that keep a spinner
            // watchable: a period shorter than a frame looks stopped or
            // backwards, and an interval of zero is a busy loop.
            "period-ms" => self.set_period_ms(value.as_millis()?),
            "frame-ms" => self.set_frame_ms(Some(value.as_millis()?)),
            _ => return Err(Mismatch::Unknown),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The interval these tests step the clock by: whatever the tree's default
    /// rate is, since that is what a spinner asking for [`Wake::Animating`]
    /// will actually be given.
    const FRAME_MS: u64 = Motion::DEFAULT_INTERVAL_MS;

    /// `animate` comes from `Widget<M>`, and a `Spinner` is not generic over
    /// the message type — so the tests pick one for it.
    fn tick(spinner: &mut Spinner, now_ms: u64) -> Animation {
        Widget::<()>::animate(spinner, now_ms)
    }

    /// A full period is exactly one revolution, and the arc lands back where it
    /// started rather than drifting by a few units a lap.
    #[test]
    fn one_period_is_one_revolution() {
        let mut spinner = Spinner::new().with_period_ms(1_000);
        // The first frame sets the epoch and moves nothing.
        tick(&mut spinner, 5_000);
        assert_eq!(spinner.angle(), 0, "the first frame must not jump");

        // Twenty frames of 50 ms is one second is one turn, back to zero.
        for frame in 1..=20 {
            tick(&mut spinner, 5_000 + frame * 50);
        }
        assert_eq!(spinner.angle(), 0, "a lap must land where it started");
    }

    /// The first frame establishes the epoch and moves nothing — whatever the
    /// application's clock happened to read when somebody asked it to spin.
    ///
    /// Asserted at a reading *below* one period on purpose. Above one, the cap
    /// that stops a long gap winding forward lands the phase on zero anyway, so
    /// a spinner that wrongly jumped by the whole clock would look correct: the
    /// first version of this test picked 5000 ms with a 1000 ms period and
    /// passed with the guard removed.
    #[test]
    fn the_first_frame_does_not_jump_by_the_applications_clock() {
        let mut spinner = Spinner::new().with_period_ms(1_000);
        tick(&mut spinner, 300);
        assert_eq!(spinner.angle(), 0, "a spinner starts where it starts");

        // And from there it turns by the time that has actually passed.
        tick(&mut spinner, 550);
        assert_eq!(spinner.angle(), TURN / 4, "250 ms of a second is a quarter");
    }

    /// The angle only ever moves forwards, and stays inside one turn however
    /// long it runs — the arithmetic mistake available to a widget that runs
    /// forever is the one that matters.
    #[test]
    fn the_angle_wraps_cleanly_and_never_leaves_the_turn() {
        let mut spinner = Spinner::new();
        tick(&mut spinner, 0);
        let mut previous = spinner.angle();
        let mut wraps = 0;
        for frame in 1..=2_000u64 {
            tick(&mut spinner, frame * FRAME_MS);
            let angle = spinner.angle();
            assert!(
                (0..TURN).contains(&angle),
                "frame {frame}: angle {angle} left the turn"
            );
            if angle < previous {
                wraps += 1;
            }
            previous = angle;
        }
        // 2 000 frames of `FRAME_MS` against the default one-second period, so
        // one lap per second of simulated time. Stated as the arithmetic rather
        // than as a number, because the frame rate is a tuning decision and this
        // test is not about what it happens to be.
        let laps = 2_000 * FRAME_MS / PERIOD_MS;
        assert!(
            wraps >= laps - laps / 10,
            "{laps} laps expected from 2000 frames of {FRAME_MS} ms, saw {wraps}"
        );
    }

    /// The period is honoured: a slower spinner turns less per frame.
    #[test]
    fn a_longer_period_turns_more_slowly() {
        let step = |period: u64| {
            let mut spinner = Spinner::new().with_period_ms(period);
            tick(&mut spinner, 0);
            tick(&mut spinner, FRAME_MS);
            spinner.angle()
        };
        let fast = step(500);
        let slow = step(4_000);
        assert!(fast > slow, "{fast} is not more per frame than {slow}");
        // One frame of a period is that fraction of the turn, whatever the frame
        // rate is: `FRAME_MS / period`.
        assert_eq!(fast, (TURN as u64 * FRAME_MS / 500) as i32);
        assert_eq!(slow, (TURN as u64 * FRAME_MS / 4_000) as i32);
    }

    /// A period below one frame is clamped: turning more than a full circle
    /// between frames looks stopped, or backwards.
    #[test]
    fn an_impossibly_short_period_is_clamped_to_a_frame() {
        for asked in [0, 1, 10, MIN_PERIOD_MS - 1] {
            let spinner = Spinner::new().with_period_ms(asked);
            assert_eq!(spinner.period_ms, MIN_PERIOD_MS, "asked for {asked}");
        }
        let mut spinner = Spinner::new();
        spinner.set_period_ms(0);
        assert_eq!(spinner.period_ms, MIN_PERIOD_MS);
    }

    /// The default is the tree's rate, and the override is a time — which is
    /// what lets one spinner differ from everything around it without any
    /// widget carrying a frame-rate constant.
    #[test]
    fn a_spinner_asks_for_the_trees_rate_unless_told_otherwise() {
        let mut spinner = Spinner::new();
        assert_eq!(tick(&mut spinner, 1_000).next, Wake::Animating);

        let mut own = Spinner::new().with_frame_ms(100);
        assert_eq!(tick(&mut own, 1_000).next, Wake::At(1_100));

        // Zero is a busy loop, not a rate.
        let mut zero = Spinner::new().with_frame_ms(0);
        assert_eq!(tick(&mut zero, 1_000).next, Wake::At(1_001));

        // And the override can be given back.
        own.set_frame_ms(None);
        assert_eq!(tick(&mut own, 1_100).next, Wake::Animating);
    }

    /// A spinner has no end state, so turning motion off stops it rather than
    /// landing it somewhere — and, importantly, drops it out of the animating
    /// set instead of leaving it asking for frames nobody will deliver.
    #[test]
    fn no_motion_stops_a_spinner_rather_than_landing_it() {
        let mut spinner = Spinner::new();
        tick(&mut spinner, 1_000);
        let angle = spinner.angle();
        assert_eq!(
            Widget::<()>::snap(&mut spinner, 2_000),
            Animation::NONE,
            "a still ring wants nothing"
        );
        assert_eq!(spinner.angle(), angle, "and it did not jump on the way");
    }

    /// It never stops asking. This is the unbounded case, and the test says so
    /// out loud so that a future change making it terminate is a decision
    /// somebody takes rather than one that happens.
    #[test]
    fn a_spinner_never_hands_the_cpu_back_on_its_own() {
        let mut spinner = Spinner::new();
        for frame in 0..200u64 {
            let animation = tick(&mut spinner, frame * FRAME_MS);
            assert_ne!(
                animation.next,
                Wake::Never,
                "frame {frame}: a spinner must keep asking"
            );
        }
    }

    /// Frames that arrive early or out of order do not move the arc backwards.
    /// The tree wakes for the most impatient animation and asks everybody, so a
    /// spinner is routinely asked before the time it requested.
    #[test]
    fn an_early_or_repeated_frame_never_rewinds_the_arc() {
        let mut spinner = Spinner::new();
        tick(&mut spinner, 1_000);
        let start = spinner.angle();

        // Asked again at the same instant: no time passed, nothing moved, and
        // the widget says so rather than claiming a repaint is owed.
        let animation = tick(&mut spinner, 1_000);
        assert_eq!(spinner.angle(), start);
        assert!(!animation.repaint, "no time passed, so nothing to repaint");

        // A clock that went backwards saturates to zero elapsed rather than
        // subtracting a turn.
        tick(&mut spinner, 500);
        assert_eq!(spinner.angle(), start, "a backwards clock must not rewind");
    }

    /// A spinner hidden for an hour and shown again resumes, rather than
    /// computing an hour of rotation.
    #[test]
    fn a_long_gap_resumes_rather_than_catching_up() {
        assert_eq!(angle_at(0, 1_000), 0);
        assert_eq!(angle_at(500, 1_000), TURN / 2);
        assert_eq!(angle_at(1_000, 1_000), 0, "a whole period is a whole turn");
        assert_eq!(angle_at(1_500, 1_000), TURN / 2, "and it wraps by phase");
        // And the widget wraps that to nothing, so it lands where it was.
        let mut spinner = Spinner::new();
        tick(&mut spinner, 0);
        tick(&mut spinner, 60 * 60 * 1_000);
        assert_eq!(spinner.angle(), 0);
    }

    /// The moving arc leaves a visible gap: a sweep of a full turn is a ring
    /// that never appears to move, however fast it spins.
    ///
    /// A `const` assertion, so it is checked when the constant is edited rather
    /// than when the suite is run.
    #[test]
    fn the_arc_leaves_a_gap_to_see_it_turn_by() {
        const { assert!(SWEEP < TURN, "a full sweep cannot be seen rotating") };
        const { assert!(SWEEP > TURN / 2, "and too short a one is a fragment") };
    }
}
