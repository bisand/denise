//! Transient notifications: appear, hold, fade, gone.
//!
//! The widget [#19] was written for. A toast is never focused, so under the old
//! rule — only the focused widget animates — it could not exist at all.
//!
//! Like the tooltip in [`crate::tooltip`], it is **not a node**, and for the
//! same kind of reasons rather than by analogy: removing itself is the whole
//! point and only the tree can remove things, two toasts must not land on top
//! of each other and no widget can see its siblings, and not being a node is
//! what makes it invisible to Tab and to hit testing without anybody
//! remembering to make it so.
//!
//! # It is the transient half of `Alert`
//!
//! [`Alert`](crate::widgets::Alert) stays exactly as it is: an **inline**
//! banner, in the layout, where the thing it is about would be. A toast is the
//! same message when there is nowhere in the layout to put it — a save that
//! succeeded, a reading that went out of range — and it goes away by itself.
//!
//! # What it costs, which is almost nothing
//!
//! A toast is mostly *idle*. It fades in, holds for a few seconds, and fades
//! out; only the fades need frames. During the hold the tree asks to be woken
//! **once**, at the moment the fade-out starts — not sixty times a second for
//! four seconds.
//!
//! That is the opposite of [`Spinner`](crate::widgets::Spinner), which looks
//! like the same kind of feature and costs a wake per frame for as long as it
//! is up.
//!
//! [#19]: https://github.com/bisand/denise/issues/19

use alloc::string::String;
use alloc::vec::Vec;

use denise::{Color, Point, Radius, Rect, Role, Size, Theme};
use denise::Pen;
use denise_text::{TextEngine, TextStyle};

use crate::motion::Motion;

/// How long a toast takes to fade in.
const FADE_IN_MS: u64 = 120;

/// How long it takes to fade out.
///
/// Longer than the fade in: something arriving should be quick, something
/// leaving should not look like it was snatched away mid-read.
const FADE_OUT_MS: u64 = 280;

/// How long it holds at full opacity, unless the caller says otherwise.
///
/// Long enough to read a short sentence twice, short enough that a panel does
/// not accumulate a wall of them.
pub(crate) const HOLD_MS: u64 = 4_000;

/// Space between a toast and the surface edge, and between two toasts.
const MARGIN: i32 = 12;

/// Space between the text and the toast's edge.
const PADDING: i32 = 10;

/// The widest a toast gets before its text wraps.
const MAX_WIDTH: i32 = 420;

/// How many show at once. Older ones are dropped rather than queued: a
/// notification nobody has seen yet is worth more than one from ten seconds
/// ago, and a panel that showed a backlog would be unreadable exactly when
/// something was going wrong.
const MAX_VISIBLE: usize = 3;

/// One notification, and when it was born.
#[derive(Clone, Debug)]
struct Note {
    text: String,
    role: Role,
    born_ms: u64,
    hold_ms: u64,
    /// The opacity this toast was last actually drawn at, or `None` before it
    /// has ever been drawn.
    ///
    /// This is what makes "is it changing?" answerable rather than guessed at.
    /// See [`Toasts::is_changing`] for the frame that went missing while the
    /// question was answered from the clock instead.
    painted_alpha: Option<u8>,
}

impl Note {
    /// How long this toast lives in total.
    #[inline]
    const fn life_ms(&self) -> u64 {
        FADE_IN_MS + self.hold_ms + FADE_OUT_MS
    }

    /// Opacity at `now_ms`, `0..=255`, and `None` once it has expired.
    ///
    /// Under [`Motion::None`] there are no fades to be part-way through, so a
    /// toast is opaque for its whole life and then gone. The *life* is
    /// unchanged: how long a notification stays readable is not a motion
    /// setting.
    fn alpha(&self, now_ms: u64, motion: Motion) -> Option<u8> {
        let age = now_ms.saturating_sub(self.born_ms);
        if age >= self.life_ms() {
            return None;
        }
        if !motion.animates() {
            return Some(255);
        }
        if age < FADE_IN_MS {
            return Some((age * 255 / FADE_IN_MS.max(1)) as u8);
        }
        let fading_at = FADE_IN_MS + self.hold_ms;
        if age < fading_at {
            return Some(255);
        }
        let out = age - fading_at;
        Some((255 - (out * 255 / FADE_OUT_MS.max(1)).min(255)) as u8)
    }

    /// When this toast next needs a frame.
    ///
    /// **The point of the whole design.** During the fades it wants the next
    /// frame; during the hold it wants exactly one wake, at the instant the
    /// fade-out starts. A toast holding for four seconds therefore costs one
    /// wake, not two hundred and forty.
    ///
    /// With no motion there are no fades, so it wants exactly one wake in its
    /// whole life: the moment it goes.
    fn next_wake(&self, now_ms: u64, motion: Motion) -> u64 {
        let Some(frame_ms) = motion.interval_ms() else {
            return self.born_ms.saturating_add(self.life_ms());
        };
        let age = now_ms.saturating_sub(self.born_ms);
        let fading_at = FADE_IN_MS + self.hold_ms;
        if age < FADE_IN_MS {
            now_ms + frame_ms
        } else if age < fading_at {
            self.born_ms + fading_at
        } else {
            now_ms + frame_ms
        }
    }
}

/// The notification stack, owned by [`Ui`](crate::Ui).
#[derive(Clone, Debug)]
pub(crate) struct Toasts {
    notes: Vec<Note>,
    /// The area the last paint actually covered.
    ///
    /// Damage needs *where the pixels are*, and by the time a toast has expired
    /// the layout no longer includes it — so asking the current stack where to
    /// repaint would miss exactly the one that just went. Remembering what was
    /// painted is the only answer that survives a toast leaving, and it is the
    /// same lesson the tooltip's damage taught: measure before the state that
    /// knows the answer is gone.
    last_painted: Option<Rect>,
    style: TextStyle,
    /// The tree's animation setting, which is what a *fading* toast asks to be
    /// redrawn at. A copy rather than a parameter on six methods, kept in step
    /// by [`Ui::set_motion`](crate::Ui::set_motion) — the stack is not a node,
    /// so nothing else would carry it here.
    motion: Motion,
}

impl Toasts {
    pub(crate) fn new() -> Self {
        Self {
            notes: Vec::new(),
            last_painted: None,
            style: TextStyle::built_in(16),
            motion: Motion::default(),
        }
    }

    /// Follows the tree's animation setting.
    pub(crate) fn set_motion(&mut self, motion: Motion) {
        self.motion = motion;
    }

    /// Adds a notification, dropping the oldest if the stack is full.
    pub(crate) fn push(&mut self, text: String, role: Role, hold_ms: u64, now_ms: u64) {
        if self.notes.len() >= MAX_VISIBLE {
            self.notes.remove(0);
        }
        self.notes.push(Note {
            text,
            role,
            born_ms: now_ms,
            hold_ms,
            painted_alpha: None,
        });
    }

    /// How many are on screen.
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.notes.len()
    }

    /// Removes everything, whether or not it had been read.
    pub(crate) fn clear(&mut self) {
        self.notes.clear();
    }

    /// Drops expired toasts. Returns `true` if any went.
    pub(crate) fn retire(&mut self, now_ms: u64) -> bool {
        let before = self.notes.len();
        self.notes
            .retain(|note| note.alpha(now_ms, self.motion).is_some());
        self.notes.len() != before
    }

    /// Whether anything on screen will look different from one frame to the
    /// next: a toast mid-fade, or one whose time is up.
    ///
    /// **The cost claim depends on this being false during the hold.** A stack
    /// that damaged itself every tick would repaint the bottom of the screen
    /// sixty times a second for four seconds to show a picture that never
    /// changed — which is the thing this whole design exists to avoid.
    ///
    /// # Ask the pixels, not the clock
    ///
    /// This used to answer from the clock: changing while `age < FADE_IN_MS`,
    /// and again once the hold was over. It is the obvious reading of "is it
    /// fading?" and it drops the frame that matters most — the one where the
    /// fade *lands*. At 120 ms and a 16 ms rate the last damaged sample is age
    /// 112, drawn at alpha 238; the frame at age 128 is the one that first
    /// draws 255, and the clock had already called the fade over.
    ///
    /// One frame of a toast being 7% dim is nothing. What it costs is the whole
    /// hold: with double buffering the *undamaged* frame still repaints the
    /// buffer it is handed, so one buffer holds a 255 toast and the other keeps
    /// the 238 one, and the panel alternates between them sixty times a second
    /// for four seconds. That is what was reported from the Pi as the toasts
    /// trembling, and it is why the answer here is now a comparison against
    /// what was last *drawn* rather than a window on the clock: the frame that
    /// lands a fade changes pixels, so it is a frame that changed, whatever the
    /// clock says. A toast holding at an opacity it has already been drawn at
    /// is still free, which is the property that had to survive.
    /// # Going is not a repaint question
    ///
    /// One thing here is *not* answered by comparing against what was drawn.
    /// A toast whose life is over has to be let go whether or not anybody ever
    /// painted it — this is the predicate [`Ui::tick`](crate::Ui::tick) gates
    /// [`retire`](Toasts::retire) on, and a tree that ticks without painting is
    /// an ordinary thing: a headless test, a surface that has not been acquired
    /// yet, an application that skipped a frame. Comparing alone said an
    /// expired toast that had never been drawn was unchanged — `None` against
    /// `None` — so it was never retired, the stack grew, and the tree never
    /// went back to sleep.
    pub(crate) fn is_changing(&self, now_ms: u64) -> bool {
        self.notes.iter().any(|note| {
            let alpha = note.alpha(now_ms, self.motion);
            alpha.is_none() || alpha != note.painted_alpha
        })
    }

    /// The soonest any toast needs a frame.
    pub(crate) fn next_wake(&self, now_ms: u64) -> Option<u64> {
        self.notes
            .iter()
            .map(|note| note.next_wake(now_ms, self.motion))
            .min()
    }

    /// Dismisses the toast containing `point`, if any.
    ///
    /// **A press on a toast must not reach what is underneath.** A toast is not
    /// a node, so nothing else will stop the press: somebody dismissing a
    /// notification would press the button it was covering, which is the
    /// dropdown bug in a new hat. Returns `true` when the press was consumed.
    pub(crate) fn dismiss_at(
        &mut self,
        point: Point,
        surface: Size,
        engine: &mut TextEngine,
        now_ms: u64,
    ) -> bool {
        let hit = self
            .placed(surface, engine, now_ms)
            .into_iter()
            .find(|(_, rect, _)| rect.contains(point))
            .map(|(index, _, _)| index);
        match hit {
            Some(index) => {
                self.notes.remove(index);
                true
            }
            None => false,
        }
    }

    /// Every visible toast: **its index in `notes`**, its rectangle and its
    /// opacity, newest nearest the corner.
    ///
    /// Stacked upwards from the bottom of the surface: a panel's content starts
    /// at the top, and on a touchscreen the bottom is where a thumb already is.
    ///
    /// The index is carried rather than implied. This walks the stack backwards
    /// *and* skips anything already expired, so a position in this list is not a
    /// position in `notes` — which is exactly the mistake the first version
    /// made twice: dismissing a toast removed a different one, and painting
    /// zipped rectangles against the wrong messages. Neither shows up with one
    /// toast on screen.
    fn placed(
        &self,
        surface: Size,
        engine: &mut TextEngine,
        now_ms: u64,
    ) -> Vec<(usize, Rect, u8)> {
        let mut out = Vec::new();
        let mut bottom = surface.height as i32 - MARGIN;
        for (index, note) in self.notes.iter().enumerate().rev() {
            let Some(alpha) = note.alpha(now_ms, self.motion) else {
                continue;
            };
            let size = measure(note, self.style, surface, engine);
            let rect = Rect::new(
                (surface.width as i32 - size.width as i32) / 2,
                bottom - size.height as i32,
                size.width as i32,
                size.height as i32,
            );
            bottom = rect.y - MARGIN;
            out.push((index, rect, alpha));
        }
        out
    }

    /// What to repaint: what the last paint covered, and what the next one
    /// will. Either alone is wrong — the first misses a toast arriving, the
    /// second misses one that has just gone.
    pub(crate) fn bounds(
        &self,
        surface: Size,
        engine: &mut TextEngine,
        now_ms: u64,
    ) -> Option<Rect> {
        let next = self
            .placed(surface, engine, now_ms)
            .into_iter()
            .map(|(_, rect, _)| rect)
            .reduce(|a, b| a.union(&b));
        match (self.last_painted, next) {
            (Some(a), Some(b)) => Some(a.union(&b)),
            (a, b) => a.or(b),
        }
    }

    /// Draws the stack.
    pub(crate) fn paint(
        &mut self,
        theme: &Theme,
        surface: Size,
        engine: &mut TextEngine,
        now_ms: u64,
        canvas: &mut Pen<'_>,
    ) {
        let placed = self.placed(surface, engine, now_ms);
        self.last_painted = placed
            .iter()
            .map(|&(_, rect, _)| rect)
            .reduce(|a, b| a.union(&b));
        // What [`Toasts::is_changing`] compares against next frame. Recorded
        // for every placed note even though the canvas is clipped to one damage
        // region at a time and this runs once per region: a note that is
        // changing has had its whole footprint damaged by `Ui::damage_toasts`,
        // so the regions of a frame cover it between them, and every call in
        // that frame records the same opacity.
        for &(index, _, alpha) in &placed {
            self.notes[index].painted_alpha = Some(alpha);
        }
        for &(index, rect, alpha) in &placed {
            let note = &self.notes[index];
            // Both colours from one pairing, and both faded together — the
            // whole reason a toast is a widget-shaped thing rather than a
            // `fill_rect` and a `draw_text` in an application.
            let (fill, content) = theme.pair(note.role);
            canvas.fill_rounded_rect(rect, theme.radius(Radius::Box), fade(fill, alpha));

            let available = rect.width - PADDING * 2;
            let line_height = engine.line_height(self.style);
            let lines: Vec<&str> = engine.wrap(self.style, &note.text, available.max(1));
            for (index, line) in lines.iter().enumerate() {
                let y = rect.y + PADDING + index as i32 * line_height;
                if y >= rect.bottom() {
                    break;
                }
                engine.draw(
                    canvas,
                    self.style,
                    Point::new(rect.x + PADDING, y),
                    line,
                    fade(content, alpha),
                );
            }
        }
    }
}

/// A colour at a fraction of its own alpha.
fn fade(color: Color, alpha: u8) -> Color {
    Color::rgba(
        color.r,
        color.g,
        color.b,
        ((color.a as u32 * alpha as u32) / 255) as u8,
    )
}

/// How big a toast is, once its text has wrapped.
fn measure(note: &Note, style: TextStyle, surface: Size, engine: &mut TextEngine) -> Size {
    let limit = MAX_WIDTH.min(surface.width as i32 - MARGIN * 2).max(1);
    let available = (limit - PADDING * 2).max(1);
    let height = engine.wrapped_height(style, &note.text, available);
    let lines: Vec<&str> = engine.wrap(style, &note.text, available);
    let widest = lines
        .iter()
        .map(|line| engine.measure_line(style, line))
        .max()
        .unwrap_or(0);
    Size::new(
        (widest + PADDING * 2).clamp(1, limit) as u32,
        (height + PADDING * 2).max(1) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SURFACE: Size = Size::new(400, 240);

    /// The tree's default rate, which is what a stack nobody has reconfigured
    /// fades at.
    const INTERVAL: u64 = Motion::DEFAULT_INTERVAL_MS;
    const MOTION: Motion = Motion::Every(INTERVAL);

    fn note(now_ms: u64) -> Note {
        Note {
            text: String::from("Lagret"),
            role: Role::Success,
            born_ms: now_ms,
            hold_ms: HOLD_MS,
            painted_alpha: None,
        }
    }

    /// **The frame that lands the fade must be a frame that repaints.**
    ///
    /// The regression this is here for: `is_changing` answered from the clock,
    /// so the sample that first draws full opacity — the one at or just past
    /// `FADE_IN_MS` — was called "not changing" and never damaged. One buffer
    /// therefore kept the last fading opacity for the whole hold while the
    /// other held 255, and a double-buffered panel alternated between them.
    /// Reported from a Pi as the toasts trembling.
    #[test]
    fn the_frame_that_lands_the_fade_is_a_frame_that_changed() {
        let mut toasts = Toasts::new();
        toasts.set_motion(MOTION);
        toasts.push(String::from("Lagret"), Role::Success, HOLD_MS, 0);

        // Walk the fade on the tree's own grid, recording what each damaged
        // frame would have drawn, exactly as `Ui::paint` does.
        let mut now = 0;
        let mut last_drawn = None;
        while now < FADE_IN_MS + INTERVAL * 4 {
            if toasts.is_changing(now) {
                last_drawn = toasts.notes[0].alpha(now, MOTION);
                toasts.notes[0].painted_alpha = last_drawn;
            }
            now += INTERVAL;
        }

        assert_eq!(
            last_drawn,
            Some(255),
            "the fade has to have been drawn at the opacity it holds at"
        );
        assert!(
            !toasts.is_changing(now),
            "and then stop asking, or the hold is not free"
        );
    }

    /// **A toast nobody ever painted still has to go.**
    ///
    /// The regression the first version of the comparison caused, and the one
    /// the unit tests above could not see because they all draw. `is_changing`
    /// gates retirement as well as damage, and comparing what would be drawn
    /// against what *was* drawn made an expired-but-never-painted toast look
    /// unchanged: `None` against `None`. The stack then grew without bound and
    /// the tree never went idle. A headless tick is not exotic — a test, a
    /// surface not yet acquired, a skipped frame.
    #[test]
    fn a_toast_that_was_never_drawn_still_expires() {
        let mut toasts = Toasts::new();
        toasts.set_motion(MOTION);
        toasts.push(String::from("Lagret"), Role::Success, HOLD_MS, 0);

        let dead = FADE_IN_MS + HOLD_MS + FADE_OUT_MS;
        assert!(
            toasts.is_changing(dead),
            "its life is over; that is a change whoever was looking"
        );
        assert!(toasts.retire(dead), "and retiring it takes it");
        assert_eq!(toasts.len(), 0);
        assert!(!toasts.is_changing(dead), "an empty stack asks for nothing");
    }

    /// The other half of that: a toast holding still costs nothing. The whole
    /// design claim rests on it.
    #[test]
    fn a_toast_that_has_been_drawn_where_it_is_asks_for_nothing() {
        let mut toasts = Toasts::new();
        toasts.set_motion(MOTION);
        toasts.push(String::from("Lagret"), Role::Success, HOLD_MS, 0);
        toasts.notes[0].painted_alpha = Some(255);

        for now in [FADE_IN_MS, FADE_IN_MS + 1_000, FADE_IN_MS + HOLD_MS - 1] {
            assert!(!toasts.is_changing(now), "still at 255 at {now} ms");
        }
        assert!(
            toasts.is_changing(FADE_IN_MS + HOLD_MS + INTERVAL),
            "and wakes again when the fade-out has moved it"
        );
    }

    /// The whole life, without anybody touching it: in, hold, out, gone.
    #[test]
    fn a_toast_fades_in_holds_and_fades_out_by_itself() {
        let note = note(1_000);
        assert_eq!(note.alpha(1_000, MOTION), Some(0), "born invisible");
        assert!(
            note.alpha(1_000 + FADE_IN_MS / 2, MOTION)
                .expect("fading in")
                > 0
        );
        assert_eq!(note.alpha(1_000 + FADE_IN_MS, MOTION), Some(255), "arrived");
        assert_eq!(
            note.alpha(1_000 + FADE_IN_MS + HOLD_MS / 2, MOTION),
            Some(255),
            "holding"
        );

        let fading = 1_000 + FADE_IN_MS + HOLD_MS + FADE_OUT_MS / 2;
        let half = note.alpha(fading, MOTION).expect("fading out");
        assert!((80..180).contains(&half), "half way out is {half}");

        assert_eq!(note.alpha(1_000 + note.life_ms(), MOTION), None, "gone");
        assert_eq!(note.alpha(u64::MAX, MOTION), None, "and stays gone");
    }

    /// **The cost claim.** A holding toast asks for one wake, at the instant it
    /// starts fading — not a frame cadence for four seconds.
    #[test]
    fn a_holding_toast_asks_for_exactly_one_wake() {
        let note = note(0);
        let fading_at = FADE_IN_MS + HOLD_MS;

        // Just after it arrives, the next wake is the whole hold away.
        let wake = note.next_wake(FADE_IN_MS, MOTION);
        assert_eq!(wake, fading_at, "a holding toast wakes once, at the fade");

        // Still one wake, most of the way through the hold.
        assert_eq!(note.next_wake(fading_at - 1, MOTION), fading_at);

        // During the fades it wants frames.
        assert_eq!(note.next_wake(0, MOTION), INTERVAL, "fading in");
        assert_eq!(
            note.next_wake(fading_at + 10, MOTION),
            fading_at + 10 + INTERVAL,
            "out"
        );
    }

    /// A tree with no toasts asks for nothing at all.
    #[test]
    fn an_empty_stack_wakes_for_nothing() {
        let toasts = Toasts::new();
        assert_eq!(toasts.next_wake(0), None);
        assert_eq!(toasts.len(), 0);
    }

    /// Expired toasts are dropped, and `retire` says whether anything went so
    /// the tree knows to repaint.
    #[test]
    fn expired_toasts_are_retired() {
        let mut toasts = Toasts::new();
        toasts.push(String::from("Lagret"), Role::Success, HOLD_MS, 0);
        assert!(!toasts.retire(100), "still alive");
        assert_eq!(toasts.len(), 1);

        assert!(toasts.retire(FADE_IN_MS + HOLD_MS + FADE_OUT_MS));
        assert_eq!(toasts.len(), 0);
        assert!(!toasts.retire(u64::MAX), "nothing left to retire");
    }

    /// Two toasts stack without overlapping, newest nearest the bottom edge.
    #[test]
    fn toasts_stack_without_overlapping() {
        let mut engine = TextEngine::new();
        let mut toasts = Toasts::new();
        toasts.push(String::from("Først"), Role::Info, HOLD_MS, 0);
        toasts.push(String::from("Så dette"), Role::Success, HOLD_MS, 0);

        let placed = toasts.placed(SURFACE, &mut engine, FADE_IN_MS);
        assert_eq!(placed.len(), 2);
        let (newest_index, newest, _) = placed[0];
        let (oldest_index, oldest, _) = placed[1];
        assert_eq!(newest_index, 1, "the newest is the last one pushed");
        assert_eq!(oldest_index, 0);
        assert!(
            newest.y > oldest.y,
            "the newest should be nearest the bottom: {newest:?} {oldest:?}"
        );
        assert!(
            oldest.bottom() <= newest.y,
            "they overlap: {oldest:?} {newest:?}"
        );
        assert!(
            newest.bottom() <= SURFACE.height as i32 - MARGIN,
            "the stack ran off the bottom"
        );
        for (_, rect, _) in &placed {
            assert!(rect.x >= 0 && rect.right() <= SURFACE.width as i32);
        }
    }

    /// The stack is capped: a panel that showed a backlog would be unreadable
    /// exactly when something was going wrong.
    #[test]
    fn the_oldest_is_dropped_when_the_stack_is_full() {
        let mut toasts = Toasts::new();
        for i in 0..MAX_VISIBLE + 2 {
            toasts.push(alloc::format!("Melding {i}"), Role::Info, HOLD_MS, 0);
        }
        assert_eq!(toasts.len(), MAX_VISIBLE);
        assert_eq!(
            toasts.notes[0].text, "Melding 2",
            "the oldest two should have gone"
        );
    }

    /// A press inside a toast dismisses it and reports that it was consumed —
    /// a press outside is not this stack's business.
    #[test]
    fn a_press_inside_a_toast_dismisses_it_and_is_consumed() {
        let mut engine = TextEngine::new();
        let mut toasts = Toasts::new();
        toasts.push(String::from("Lagret"), Role::Success, HOLD_MS, 0);
        let (_, rect, _) = toasts.placed(SURFACE, &mut engine, FADE_IN_MS)[0];

        let outside = Point::new(rect.x - 5, rect.y - 5);
        assert!(!toasts.dismiss_at(outside, SURFACE, &mut engine, FADE_IN_MS));
        assert_eq!(toasts.len(), 1, "and it is still there");

        let inside = Point::new(rect.x + 2, rect.y + 2);
        assert!(toasts.dismiss_at(inside, SURFACE, &mut engine, FADE_IN_MS));
        assert_eq!(toasts.len(), 0);
    }

    /// Dismissing the right one of several. The first version removed by the
    /// position in the *placed* list, which walks backwards — so tapping the
    /// newest removed the oldest. Invisible with one toast on screen.
    #[test]
    fn dismissing_removes_the_toast_that_was_pressed() {
        let mut engine = TextEngine::new();
        let mut toasts = Toasts::new();
        toasts.push(String::from("Først"), Role::Info, HOLD_MS, 0);
        toasts.push(String::from("Andre"), Role::Success, HOLD_MS, 0);
        toasts.push(String::from("Tredje"), Role::Error, HOLD_MS, 0);

        // The newest is nearest the bottom edge, and is `notes[2]`.
        let placed = toasts.placed(SURFACE, &mut engine, FADE_IN_MS);
        let (_, newest, _) = placed[0];
        assert!(toasts.dismiss_at(
            Point::new(newest.x + 2, newest.y + 2),
            SURFACE,
            &mut engine,
            FADE_IN_MS
        ));
        assert_eq!(toasts.len(), 2);
        assert_eq!(
            [toasts.notes[0].text.as_str(), toasts.notes[1].text.as_str()],
            ["Først", "Andre"],
            "the wrong toast was dismissed"
        );

        // And the oldest, now at the top of the stack.
        let placed = toasts.placed(SURFACE, &mut engine, FADE_IN_MS);
        let (_, oldest, _) = placed[1];
        assert!(toasts.dismiss_at(
            Point::new(oldest.x + 2, oldest.y + 2),
            SURFACE,
            &mut engine,
            FADE_IN_MS
        ));
        assert_eq!(toasts.notes[0].text, "Andre");
    }

    /// An expired toast still in the list must not shift the ones after it out
    /// of alignment with their messages — `placed` skips it, so it carries the
    /// index rather than implying one.
    #[test]
    fn an_expired_toast_does_not_misalign_the_rest() {
        let mut engine = TextEngine::new();
        let mut toasts = Toasts::new();
        toasts.push(String::from("Gammel"), Role::Info, 0, 0);
        toasts.push(String::from("Ny"), Role::Success, HOLD_MS, 0);

        // The first has expired; the second has not.
        let late = FADE_IN_MS + FADE_OUT_MS + 1;
        let placed = toasts.placed(SURFACE, &mut engine, late);
        assert_eq!(placed.len(), 1, "only the live one is placed");
        assert_eq!(placed[0].0, 1, "and it knows which message it is");
    }

    /// Long text wraps rather than running off the surface.
    #[test]
    fn a_long_message_wraps_instead_of_overflowing() {
        let mut engine = TextEngine::new();
        let short = measure(&note(0), TextStyle::built_in(16), SURFACE, &mut engine);
        let long = Note {
            text: String::from("Kunne ikke lagre fordi disken er full og det er ingen plass igjen"),
            ..note(0)
        };
        let wrapped = measure(&long, TextStyle::built_in(16), SURFACE, &mut engine);
        assert!(wrapped.height > short.height, "it did not wrap");
        assert!(
            wrapped.width as i32 <= SURFACE.width as i32 - MARGIN * 2,
            "it is wider than the surface allows: {wrapped:?}"
        );
    }

    /// Fading scales alpha and leaves the colour alone, so a toast does not
    /// change hue as it goes.
    #[test]
    fn fading_touches_only_the_alpha() {
        let colour = Color::rgba(200, 100, 50, 255);
        assert_eq!(fade(colour, 255), colour);
        let half = fade(colour, 128);
        assert_eq!((half.r, half.g, half.b), (200, 100, 50));
        assert!(half.a < colour.a && half.a > 0);
        assert_eq!(fade(colour, 0).a, 0);
    }
}
