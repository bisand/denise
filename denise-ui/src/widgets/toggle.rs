//! The same boolean as a checkbox, with a different affordance.

use alloc::string::String;

use denise::{ElementState, InputEvent, KeyCode, Rect, Role, Theme};
use denise_render::Pen;
use denise_text::{TextEngine, TextStyle};

use crate::motion::Wake;
use crate::widget::{
    Animation, Event, EventCtx, Handled, MeasureCtx, Measured, Offer, PaintCtx, VisualState, Widget,
};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Payload, Property, PropertyKind, ROLES, Value,
};
use crate::widgets::style::{Align, draw_aligned, focus_ring, interactive_pair};

/// How long the knob takes to cross, in milliseconds.
///
/// Short enough that nobody waits for it and long enough to read as movement
/// rather than as a dropped frame.
///
/// A **duration**, not a frame rate: [`Motion`](crate::Motion) decides how many
/// positions the knob is drawn in on the way, and never how long it takes.
const TRAVEL_MS: u64 = 120;

/// Knob positions are fixed-point over this range, so the whole widget is
/// integer arithmetic — `denise-ui` is `no_std` and the rasteriser is integer
/// throughout.
const SCALE: i32 = 1000;

/// A switch: a track, a knob, and a boolean.
///
/// Reads better than a [`Checkbox`](super::Checkbox) at arm's length, which is
/// what a wall panel is. Identical semantics otherwise, including the message
/// being a function of the **new** value:
///
/// ```
/// # use denise_ui::Toggle;
/// enum Message { Muted(bool) }
/// Toggle::new("Mute", Message::Muted);
/// ```
///
/// # The knob slides
///
/// Flipping the switch requests animation for the 120 ms crossing and then the
/// widget stops asking — the bounded-transition contract described on
/// [`Widget::animate`]. Focus is not involved: the knob finishes its crossing
/// whether or not focus moves away mid-slide, which is what retired the
/// `FocusLost`-snaps-the-knob workaround this widget shipped with.
///
/// The animation is still a courtesy, not a mechanism: nothing about the value
/// depends on a frame arriving, and a `Toggle` on a tree whose application
/// never calls [`Ui::tick`](crate::Ui::tick) is merely a switch that jumps.
#[derive(Clone, Debug)]
pub struct Toggle<M> {
    label: String,
    checked: bool,
    /// Knob position when the current transition started, `0..=SCALE`.
    from: i32,
    /// When the current transition started, in the [`crate::Ui::tick`] clock.
    started_ms: u64,
    message: Option<fn(bool) -> M>,
    role: Role,
    style: TextStyle,
}

impl<M> Toggle<M> {
    /// An off switch whose message is built from the value it changes to.
    pub fn new(label: impl Into<String>, message: fn(bool) -> M) -> Self {
        Self {
            label: label.into(),
            checked: false,
            from: 0,
            started_ms: 0,
            message: Some(message),
            role: Role::Primary,
            style: TextStyle::built_in(16),
        }
    }

    /// A switch that emits nothing, for a value the application reads rather
    /// than reacts to.
    pub fn inert(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            checked: false,
            from: 0,
            started_ms: 0,
            message: None,
            role: Role::Primary,
            style: TextStyle::built_in(16),
        }
    }

    /// Sets the initial value. The knob starts there rather than sliding into it.
    pub fn with_checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self.from = Self::target(checked);
        self
    }

    /// Sets the colour role of the track when on.
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

    /// Whether the switch is on.
    #[inline]
    pub const fn checked(&self) -> bool {
        self.checked
    }

    /// Sets the value **without emitting anything, and without animating**.
    ///
    /// Silent for the same reason [`Checkbox::set_checked`] is: the message
    /// reports what a person did, and an application that assigned here and got
    /// its own message back would either loop or have to guard against itself.
    ///
    /// It does not animate because a setter has no way to request frames — the
    /// tree grants them through an event or [`crate::Ui::request_animation`],
    /// and a widget cannot reach the tree from here. An application that wants
    /// the slide can call `request_animation` itself; the default is the honest
    /// jump.
    ///
    /// [`Checkbox::set_checked`]: super::Checkbox::set_checked
    pub fn set_checked(&mut self, checked: bool) {
        self.checked = checked;
        self.from = Self::target(checked);
        self.started_ms = 0;
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
    pub fn set_style(&mut self, style: TextStyle) {
        self.style = style;
    }

    /// Width this toggle needs for its track, its gap and its label.
    pub fn preferred_width(&self, theme: &Theme, engine: &mut TextEngine) -> i32 {
        let height = theme.metrics.size_selector;
        let track = track_width(height);
        if self.label.is_empty() {
            track
        } else {
            track + gap(height) + engine.measure_line(self.style, &self.label)
        }
    }

    #[inline]
    const fn target(checked: bool) -> i32 {
        if checked { SCALE } else { 0 }
    }

    /// Where the knob is at `now_ms`, `0..=SCALE`.
    ///
    /// Interpolated from where it was when the transition started rather than
    /// accumulated per frame, so a toggle flipped back mid-slide reverses from
    /// where it actually is and a dropped frame costs nothing.
    fn position(&self, now_ms: u64) -> i32 {
        let target = Self::target(self.checked);
        if self.from == target {
            return target;
        }
        let elapsed = now_ms.saturating_sub(self.started_ms);
        if elapsed >= TRAVEL_MS {
            return target;
        }
        let progress = (elapsed as i32) * SCALE / (TRAVEL_MS as i32);
        self.from + (target - self.from) * progress / SCALE
    }

    /// Whether the knob is still moving at `now_ms`.
    #[inline]
    fn moving(&self, now_ms: u64) -> bool {
        self.from != Self::target(self.checked)
            && now_ms.saturating_sub(self.started_ms) < TRAVEL_MS
    }

    /// Ends any transition immediately, wherever it had got to.
    ///
    /// The arrival itself. [`Widget::snap`] is the tree asking for it to happen
    /// now.
    fn settle(&mut self) {
        self.from = Self::target(self.checked);
    }
}

/// Space between the track and its label.
#[inline]
const fn gap(height: i32) -> i32 {
    if height < 2 { 1 } else { height / 2 }
}

/// A switch is wider than it is tall, or it is a circle with a knob in it.
#[inline]
const fn track_width(height: i32) -> i32 {
    let width = height * 9 / 5;
    if width < height + 2 {
        height + 2
    } else {
        width
    }
}

/// The track: a stadium at the leading edge, centred vertically.
fn track_rect(bounds: Rect, theme: &Theme) -> Rect {
    let mut height = theme.metrics.size_selector.min(bounds.height);
    if height < 1 {
        height = 1;
    }
    let mut width = track_width(height);
    if width > bounds.width && bounds.width >= 1 {
        width = bounds.width;
    }
    Rect::new(
        bounds.x,
        bounds.y + (bounds.height - height) / 2,
        width,
        height,
    )
}

/// The knob, for a track and a position in `0..=SCALE`.
fn knob_rect(track: Rect, position: i32) -> Rect {
    // Inset so the knob sits *in* the track rather than filling it. An eighth of
    // the height keeps the proportion at every selector metric.
    let inset = (track.height / 8).max(1);
    let size = (track.height - inset * 2).max(1);
    let travel = (track.width - inset * 2 - size).max(0);
    let x = track.x + inset + travel * position.clamp(0, SCALE) / SCALE;
    Rect::new(x, track.y + inset, size, size)
}

impl<M: 'static> Widget<M> for Toggle<M> {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn measure(&self, ctx: &mut MeasureCtx<'_>, _offered: Offer) -> Measured {
        Measured::both(
            self.preferred_width(ctx.theme, ctx.text),
            ctx.theme.metrics.size_selector.max(1),
        )
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        let track = track_rect(ctx.bounds, ctx.theme);
        // A stadium: the radius is half the height, whatever the theme's selector
        // radius says. A switch with square-ish corners reads as a progress bar.
        let radius = track.height / 2;

        // The label stays plain text on the panel rather than taking the track's
        // role colour; reading it through the pairing is what mutes it when disabled.
        let on_surface = interactive_pair(ctx.theme, Role::Base100, ctx.state).1;

        // Track and knob come out of the same pairing, which is what makes the
        // knob readable rather than hopeful: `content_of` is defined as "the
        // colour to draw on top of this role", and a knob is a thing drawn on top
        // of the track. A fixed knob colour cannot work — `Base100` on `Base300`
        // is 1.38:1 on the light theme, two light surfaces on top of each other,
        // and `the_knob_is_visible_against_both_track_colours_in_every_theme` is
        // what caught it.
        //
        // Both follow the *value* rather than the knob's position, so a toggle
        // mid-slide already shows where it is going.
        let (track_color, knob_color) = if self.checked {
            interactive_pair(ctx.theme, self.role, ctx.state)
        } else {
            interactive_pair(ctx.theme, Role::Base300, ctx.state)
        };
        canvas.fill_rounded_rect(track, radius, track_color);

        let knob = knob_rect(track, self.position(ctx.now_ms));
        canvas.fill_rounded_rect(knob, knob.height / 2, knob_color);

        if ctx.state.contains(VisualState::FOCUSED) {
            focus_ring(
                ctx.theme,
                ctx.bounds,
                ctx.theme.radius(denise::Radius::Field),
                canvas,
            );
        }

        if self.label.is_empty() {
            return;
        }
        let text = Rect::from_edges(
            track.right() + gap(track.height),
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
            // Space, not Enter — Enter belongs to the form's default action. Same
            // rule as `Checkbox`, and for the same reason.
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

        // From wherever the knob actually is, so flipping it back mid-slide
        // reverses smoothly instead of jumping to the far end first.
        self.from = self.position(ctx.now_ms);
        self.started_ms = ctx.now_ms;
        self.checked = !self.checked;
        // Frames for the crossing, and only the crossing: `animate` answers
        // `None` the moment the knob arrives.
        ctx.request_animation();
        if let Some(message) = self.message {
            ctx.emit(message(self.checked));
        }
        Handled::Yes
    }

    fn animate(&mut self, now_ms: u64) -> Animation {
        if !self.moving(now_ms) {
            // Arrived. Collapsing `from` onto the target is what makes the next
            // `moving` cheap and, more importantly, stops this widget asking for
            // another frame — a toggle that kept requesting wakes would hold a
            // kiosk's CPU awake for the life of the device.
            //
            // **The arrival is a frame, and it used to be dropped.** `moving`
            // goes false the moment `elapsed` reaches `TRAVEL_MS`, and that is
            // the first tick on which `position` answers `target` — so the last
            // frame anyone painted had the knob a step short of the end. On a
            // double-buffered display that does not merely look imprecise: one
            // buffer holds the knob short and the other holds it home, and a
            // panel that keeps presenting alternates between them for as long
            // as the toggle is on screen. It was reported from a Pi as flicker
            // on the control under the pointer.
            //
            // So arriving repaints, and only settling again is free. Which is
            // what `snap` next door had right all along: `repaint: moving`.
            let arriving = self.from != Self::target(self.checked);
            self.settle();
            return Animation {
                repaint: arriving,
                next: Wake::Never,
            };
        }
        // Moving, at whatever rate the tree animates at. The crossing still
        // takes `TRAVEL_MS` either way: a coarser rate draws it in fewer
        // positions, it does not draw it more slowly.
        Animation::MOVING
    }

    /// The knob arrives at once. A switch is the clearest case for reduced
    /// motion: the slide was always a courtesy, and the value never depended on
    /// it.
    fn snap(&mut self, now_ms: u64) -> Animation {
        let moving = self.moving(now_ms);
        self.settle();
        Animation {
            repaint: moving,
            next: Wake::Never,
        }
    }

    fn accepts_pointer(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        true
    }
}

impl<M> Describe for Toggle<M> {
    const KIND: &'static str = "toggle";
    const DOC: &'static str = "The same on-or-off as a checkbox, shaped like a switch.";
    const GROUP: Group = Group::Input;
    const ICON: &'static denise_render::icon::Icon = &super::icons::TOGGLE;

    const PROPERTIES: &'static [Property] = &[
        Property::new("text", PropertyKind::Text, "The label beside the track."),
        Property::new("checked", PropertyKind::Bool, "Whether the switch is on."),
        Property::new(
            "on-change",
            PropertyKind::Message(Payload::Bool),
            "The message built from the value the switch changes to. Omitted, the toggle is inert.",
        ),
        Property::new(
            "role",
            PropertyKind::Enum(ROLES),
            "Colour role of the track when on. The knob comes from the theme's pairing, so it stays visible against it.",
        ),
        Property::new(
            "size",
            PropertyKind::Int { min: 6, max: 96 },
            "Label text size in logical pixels. The track itself is a theme metric.",
        )
        .in_pixels(),
    ];

    fn get(&self, name: &str) -> Option<Value> {
        Some(match name {
            "text" => Value::text(self.label.as_str()),
            "checked" => Value::Bool(self.checked),
            // A `fn(bool) -> M` cannot be reported as a `Value`. See the
            // `describe` module docs.
            "on-change" => return None,
            "role" => Value::role(self.role),
            "size" => Value::Int(i32::from(self.style.size_px)),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            "text" => self.label = value.as_text()?,
            // The setter, which lands the knob where the value says rather than
            // sliding it there. Assigning `checked` alone would leave `from` at
            // the other end, and a form that opened with three switches gliding
            // into place would be animating a state nobody changed.
            "checked" => self.set_checked(value.as_bool()?),
            "on-change" => return Err(Mismatch::Supplied),
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

    /// A switch has to be wider than it is tall, or it is a circle with a knob
    /// rattling around inside it.
    #[test]
    fn the_track_is_a_stadium_at_the_leading_edge() {
        let bounds = Rect::new(10, 20, 300, 40);
        let track = track_rect(bounds, &theme::DARK);

        assert_eq!(track.height, theme::DARK.metrics.size_selector);
        assert!(track.width > track.height, "a switch is not a circle");
        assert_eq!(track.x, bounds.x);
        assert_eq!(
            track.y + track.height / 2,
            bounds.y + bounds.height / 2,
            "centred against the label beside it"
        );
    }

    /// Degenerate bounds still have to produce something drawable. A knob with
    /// zero or negative size reaches `fill_rounded_rect` as an inverted rectangle.
    #[test]
    fn degenerate_bounds_still_give_a_drawable_track_and_knob() {
        for bounds in [
            Rect::new(0, 0, 0, 0),
            Rect::new(0, 0, 1, 40),
            Rect::new(0, 0, 300, 1),
            Rect::new(0, 0, 4, 4),
        ] {
            let track = track_rect(bounds, &theme::DARK);
            assert!(track.height >= 1, "{bounds:?} gave no track");
            for position in [0, SCALE / 2, SCALE] {
                let knob = knob_rect(track, position);
                assert!(
                    knob.width >= 1 && knob.height >= 1,
                    "{bounds:?} @ {position}"
                );
            }
        }
    }

    /// The knob stays inside the track at both ends, which is the whole of what
    /// the travel arithmetic has to get right.
    #[test]
    fn the_knob_stays_inside_the_track_across_its_whole_travel() {
        for metrics in [theme::Metrics::DEFAULT, theme::Metrics::TOUCH] {
            let theme = theme::DARK.with_metrics(metrics);
            let track = track_rect(Rect::new(0, 0, 300, 60), &theme);
            let (mut previous, mut moved) = (i32::MIN, false);

            for position in 0..=SCALE {
                let knob = knob_rect(track, position);
                assert!(
                    track.contains_rect(&knob),
                    "{metrics:?} @ {position}: {knob:?} escaped {track:?}"
                );
                if knob.x > previous && previous != i32::MIN {
                    moved = true;
                }
                assert!(knob.x >= previous, "the knob went backwards");
                previous = knob.x;
            }
            assert!(moved, "the knob never moved at all");
        }
    }

    /// Out-of-range positions are clamped rather than producing a knob outside
    /// the track. Nothing should pass one, which is exactly why it is worth
    /// pinning: the clamp is invisible until it is missing.
    #[test]
    fn a_position_outside_the_range_is_clamped() {
        let track = track_rect(Rect::new(0, 0, 300, 40), &theme::DARK);
        assert_eq!(knob_rect(track, -5000), knob_rect(track, 0));
        assert_eq!(knob_rect(track, SCALE * 3), knob_rect(track, SCALE));
    }

    /// The interpolation: starts where it was, ends where it is going, and is
    /// somewhere in between along the way.
    #[test]
    fn the_knob_crosses_over_the_travel_time_and_then_stops() {
        let mut toggle: Toggle<bool> = Toggle::new("Mute", |on| on);
        toggle.checked = true;
        toggle.from = 0;
        toggle.started_ms = 1_000;

        assert_eq!(toggle.position(1_000), 0);
        let middle = toggle.position(1_000 + TRAVEL_MS / 2);
        assert!(
            middle > 0 && middle < SCALE,
            "{middle} is not between the ends"
        );
        assert_eq!(toggle.position(1_000 + TRAVEL_MS), SCALE);
        assert_eq!(
            toggle.position(9_999_999),
            SCALE,
            "and stays there rather than overshooting"
        );
        assert!(!toggle.moving(1_000 + TRAVEL_MS));
    }

    /// **The frame that lands the crossing must be a frame that repaints.**
    ///
    /// The regression this is here for. `moving` goes false the instant
    /// `elapsed` reaches `TRAVEL_MS`, and that same instant is the first at
    /// which `position` answers `SCALE` — so the arrival was a frame nobody
    /// painted, and every frame before it had the knob short of the end. One
    /// buffer of a double-buffered panel therefore kept the knob short while
    /// the other had it home, and a display that keeps presenting alternates
    /// between the two. It was reported from a Pi as flicker on the control
    /// under the pointer, and it is the same defect the toast stack had.
    #[test]
    fn the_frame_that_lands_the_crossing_repaints() {
        let mut toggle: Toggle<bool> = Toggle::new("Mute", |on| on);
        toggle.checked = true;
        toggle.from = 0;
        toggle.started_ms = 1_000;

        let last_moving = toggle.animate(1_000 + TRAVEL_MS - 1);
        assert!(last_moving.repaint, "still crossing");
        assert!(
            toggle.position(1_000 + TRAVEL_MS - 1) < SCALE,
            "and short of the end, which is what makes the next frame matter"
        );

        let landing = toggle.animate(1_000 + TRAVEL_MS);
        assert!(
            landing.repaint,
            "the knob reached the end on this frame; somebody has to draw it there"
        );
        assert_eq!(landing.next, Wake::Never, "and then stop asking");
    }

    /// The other half: a toggle sitting still is free. Asking for a repaint
    /// every tick would hold a kiosk's CPU awake for the life of the device,
    /// which is the cost the arrival must not be paid for with.
    #[test]
    fn a_settled_toggle_asks_for_nothing() {
        let mut toggle: Toggle<bool> = Toggle::new("Mute", |on| on);
        toggle.checked = true;
        toggle.from = 0;
        toggle.started_ms = 1_000;
        toggle.animate(1_000 + TRAVEL_MS);

        for now in [1_000 + TRAVEL_MS, 2_000, 9_999_999] {
            let idle = toggle.animate(now);
            assert!(!idle.repaint, "nothing changed at {now} ms");
            assert_eq!(idle.next, Wake::Never);
        }
    }

    /// A clock that goes backwards — a host that resets its epoch, or a `tick`
    /// arriving out of order — must not produce a negative elapsed time.
    #[test]
    fn a_clock_that_goes_backwards_does_not_underflow() {
        let mut toggle: Toggle<bool> = Toggle::new("Mute", |on| on);
        toggle.checked = true;
        toggle.from = 0;
        toggle.started_ms = 5_000;
        assert_eq!(toggle.position(1), 0, "before the start is the start");
    }

    /// Flipping back mid-slide reverses from where the knob actually is, rather
    /// than snapping to the far end and starting from there.
    #[test]
    fn flipping_back_halfway_reverses_from_where_it_got_to() {
        let mut toggle: Toggle<bool> = Toggle::new("Mute", |on| on);
        toggle.checked = true;
        toggle.from = 0;
        toggle.started_ms = 0;

        let halfway = toggle.position(TRAVEL_MS / 2);
        // What `on_event` does when it is toggled again.
        toggle.from = halfway;
        toggle.started_ms = TRAVEL_MS / 2;
        toggle.checked = false;

        assert_eq!(toggle.position(TRAVEL_MS / 2), halfway, "no jump");
        assert_eq!(toggle.position(TRAVEL_MS / 2 + TRAVEL_MS), 0);
    }

    /// Assigning is silent *and* immediate: a setter cannot request frames, so
    /// the honest picture is the one that needs none.
    #[test]
    fn setting_the_value_programmatically_is_silent_and_lands_immediately() {
        let mut toggle: Toggle<bool> = Toggle::new("Mute", |on| on);
        toggle.set_checked(true);
        assert!(toggle.checked());
        assert_eq!(toggle.position(0), SCALE);
        assert!(!toggle.moving(0));
    }

    /// A knob that vanishes into its track is a switch with no visible state at
    /// all, and it vanishes in exactly one theme rather than all of them — which
    /// is the kind of thing that ships.
    ///
    /// This is what rejected the obvious rule of a fixed `Base100` knob: on the
    /// light theme that is 1.38:1 against a `Base300` track, two light surfaces
    /// stacked. Pairing the knob with the track through `interactive_pair` makes
    /// the guarantee structural instead.
    #[test]
    fn the_knob_is_visible_against_its_track_in_every_theme_and_state() {
        use denise::theme::{AA_LARGE, contrast_x100};

        for theme in Theme::BUILT_IN {
            for state in [
                VisualState::NONE,
                VisualState::HOVERED,
                VisualState::PRESSED,
                VisualState::DISABLED,
                VisualState::FOCUSED,
            ] {
                for (name, role) in [("off", Role::Base300), ("on", Role::Primary)] {
                    let (track, knob) = interactive_pair(&theme, role, state);
                    let ratio = contrast_x100(track, knob);
                    assert!(
                        ratio >= AA_LARGE,
                        "{} {name} {state:?}: knob against track is {ratio}, \
                         floor is {AA_LARGE}",
                        theme.name
                    );
                }
            }
        }
    }
}
