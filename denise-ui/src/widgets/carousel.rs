//! Pictures shown one at a time, sliding between them.

use alloc::vec::Vec;

use denise::{ElementState, InputEvent, KeyCode, Point, Rect, Role, Size};
use denise::Pen;

use crate::motion::Wake;
use crate::widget::{Animation, Event, EventCtx, Handled, PaintCtx, VisualState, Widget};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Payload, Property, PropertyKind, ROLES, Value,
};
use crate::widgets::image::{Fit, Image};
use crate::widgets::style::{focus_ring, interactive_pair};

/// How long a slide takes. A quarter second reads as motion without making a
/// person wait for it.
///
/// A **duration**: [`Motion`](crate::Motion) decides how often the slide is
/// sampled on the way, never how long it lasts.
const SLIDE_MS: u64 = 250;

/// Dragging past this fraction of the width commits to the next page.
const COMMIT_DIVISOR: i32 = 4;

/// One whole page width, in the fixed-point fraction slides are measured in.
///
/// A slide's displacement is stored as a fraction of the width rather than in
/// pixels, because [`Widget::animate`] has no geometry: the advance clock
/// starts a slide without ever having seen the widget's rectangle, and a
/// fraction lets paint — which has the rectangle — do the multiply.
const WHOLE: i32 = 1024;

/// The indicator dots' radius, and the spacing between their centres.
const DOT: i32 = 4;
const DOT_GAP: i32 = 14;

/// What the carousel is doing between input events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Showing the current page, waiting for input or the advance clock.
    Still,
    /// A finger holds the pages displaced by a fraction of the width,
    /// [`WHOLE`] being one page.
    Dragging { fraction: i32 },
    /// Sliding from `fraction` back to rest on the (already updated) current
    /// page, having started at `from_ms`.
    Sliding { fraction: i32, from_ms: u64 },
}

/// Pictures shown one at a time in one rectangle, sliding between them.
///
/// ```
/// # use denise_ui::widgets::Carousel;
/// # use denise::Size;
/// # let (sunset, harbour) = (vec![0u32; 4], vec![0u32; 4]);
/// enum Message { Page(usize) }
/// Carousel::new(Message::Page)
///     .with_picture(sunset, Size::new(640, 480))
///     .with_picture(harbour, Size::new(640, 480))
///     .auto_advance(8_000);
/// ```
///
/// The signage rotator: swipe or drag changes the page, arrow keys change it
/// from the keyboard, and [`auto_advance`](Carousel::auto_advance) turns it
/// into the idle-screen photo loop. Pages are *pictures* — the premultiplied
/// buffers [`Image`] takes, each with its own [`Fit`] — because a carousel of
/// arbitrary widgets would need this widget to own nodes, and `EventCtx`
/// deliberately cannot. A carousel of mixed content is composed the other way
/// round: [`Tabs`](super::Tabs) without visible tabs, node visibility swapped
/// on the message.
///
/// # It wraps
///
/// A rotator is a cycle by definition — the advance clock has to come round —
/// so the keyboard wraps too: [`RadioGroup`](super::RadioGroup)'s convention,
/// not [`List`](super::List)'s.
///
/// # What it costs
///
/// Idle with no advance clock: nothing. Holding on a page with one: **one
/// wake per interval**, the toast arrangement — [`animate`](Widget::animate)
/// answers the deadline and nothing repaints until it fires. Frames are spent
/// only during the quarter-second slide. Like [`Spinner`](super::Spinner) it
/// does not start itself: the advance clock runs once the application calls
/// [`Ui::request_animation`](crate::Ui::request_animation), so nothing
/// rotates merely because a screen exists.
///
/// # The settle message
///
/// `fn(usize) -> M` is emitted when a *person* lands the carousel on a page —
/// a committed swipe, an arrow key — and once per arrival, not per frame. A
/// drag that springs back emits nothing, and neither does the advance clock:
/// the clock is the machine talking to itself, and a message reports what a
/// person did — the rule every silent setter in this crate follows. An
/// application that needs the shown page reads [`current`](Carousel::current).
#[derive(Clone, Debug)]
pub struct Carousel<M> {
    pages: Vec<Image>,
    current: usize,
    phase: Phase,
    /// Where a drag started, and the width it is measured against.
    grip: Option<(Point, i32)>,
    /// The advance interval, if the application asked for one.
    advance_ms: Option<u64>,
    /// When the current hold began, against `tick`'s clock.
    held_since: u64,
    message: Option<fn(usize) -> M>,
    role: Role,
}

impl<M> Carousel<M> {
    /// An empty carousel, reporting arrivals through `message`.
    pub fn new(message: fn(usize) -> M) -> Self {
        Self {
            pages: Vec::new(),
            current: 0,
            phase: Phase::Still,
            grip: None,
            advance_ms: None,
            held_since: 0,
            message: Some(message),
            role: Role::Primary,
        }
    }

    /// A carousel that emits nothing — the pure signage case, where nobody is
    /// listening and the pictures simply rotate.
    pub fn inert() -> Self {
        Self {
            pages: Vec::new(),
            current: 0,
            phase: Phase::Still,
            grip: None,
            advance_ms: None,
            held_since: 0,
            message: None,
            role: Role::Primary,
        }
    }

    /// Adds a page: premultiplied `0xAARRGGBB` pixels, [`Image`]'s contract,
    /// shown with [`Fit::Cover`].
    pub fn with_picture(mut self, pixels: Vec<u32>, size: Size) -> Self {
        self.pages
            .push(Image::new(pixels, size).with_fit(Fit::Cover));
        self
    }

    /// Adds a page with its own [`Fit`].
    pub fn with_picture_fit(mut self, pixels: Vec<u32>, size: Size, fit: Fit) -> Self {
        self.pages.push(Image::new(pixels, size).with_fit(fit));
        self
    }

    /// Advances to the next page every `interval_ms` — once the application
    /// starts the clock with
    /// [`Ui::request_animation`](crate::Ui::request_animation).
    ///
    /// Floored at twice the slide, because an interval the slide cannot keep
    /// up with is a carousel that never rests.
    pub fn auto_advance(mut self, interval_ms: u64) -> Self {
        self.advance_ms = Some(interval_ms.max(SLIDE_MS * 2));
        self
    }

    /// Sets the colour role of the current page's dot and the focus ring.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// The page currently shown, or arriving.
    #[inline]
    pub const fn current(&self) -> usize {
        self.current
    }

    /// How many pages there are.
    #[inline]
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Shows a page immediately, without sliding and without emitting — the
    /// application writing state, like every setter here. Out of range does
    /// nothing.
    pub fn set_current(&mut self, index: usize) {
        if index < self.pages.len() {
            self.current = index;
            self.phase = Phase::Still;
        }
    }

    /// Appends a page after construction, reporting its index.
    pub fn push_picture(&mut self, pixels: Vec<u32>, size: Size) -> usize {
        self.pages
            .push(Image::new(pixels, size).with_fit(Fit::Cover));
        self.pages.len() - 1
    }

    /// When the advance clock next wants waking, or [`Wake::Never`] if there is
    /// no clock.
    ///
    /// A deadline rather than a rate, so it is untouched by
    /// [`Motion`](crate::Motion): a carousel set to advance every eight seconds
    /// advances every eight seconds at any frame rate, and at none.
    ///
    /// Saturating, because the clock is the application's and every deadline
    /// here is derived from it.
    fn advance_wake(&self, now_ms: u64) -> Wake {
        match self.advance_ms {
            Some(interval) => Wake::At(now_ms.saturating_add(interval)),
            None => Wake::Never,
        }
    }

    /// The page `steps` away, wrapping — a rotator is a cycle.
    fn neighbour(&self, steps: i32) -> usize {
        let count = self.pages.len().max(1) as i32;
        (self.current as i32 + steps).rem_euclid(count) as usize
    }

    /// The current displacement as a fraction of the width, [`WHOLE`] being
    /// one page.
    fn fraction_at(&self, now_ms: u64) -> i32 {
        match self.phase {
            Phase::Still => 0,
            Phase::Dragging { fraction } => fraction,
            Phase::Sliding { fraction, from_ms } => {
                slide_fraction(fraction, now_ms.saturating_sub(from_ms))
            }
        }
    }

    /// Lands on `target` and slides in from `fraction`, emitting the arrival.
    ///
    /// The current page is updated *now* and the slide runs from a
    /// displacement back to rest — which is what makes an interrupted slide
    /// land somewhere honest rather than between pages.
    /// The advance clock is *not* reset here, and deliberately: every slide
    /// ends in [`Widget::animate`]'s landing, which restarts the hold from the
    /// moment of arrival. An earlier version reset it here too, and a mutation
    /// removing that changed nothing observable — the landing was already
    /// doing the work — so it came out, the `label_box` rule.
    fn arrive(&mut self, target: usize, fraction: i32, ctx: &mut EventCtx<'_, M>) {
        self.current = target;
        self.phase = Phase::Sliding {
            fraction,
            from_ms: ctx.now_ms,
        };
        if let Some(message) = self.message {
            ctx.emit(message(target));
        }
        ctx.request_animation();
    }
}

/// Where a slide that started displaced by `fraction` has got to, `elapsed`
/// in. Linear: on the panels this ships to, a quarter-second linear slide is
/// indistinguishable from an eased one and costs no curve table.
fn slide_fraction(fraction: i32, elapsed: u64) -> i32 {
    if elapsed >= SLIDE_MS {
        return 0;
    }
    let remaining = (SLIDE_MS - elapsed) as i64;
    (i64::from(fraction) * remaining / SLIDE_MS as i64) as i32
}

impl<M: 'static> Widget<M> for Carousel<M> {
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
        // The backdrop: letterboxing, and the ground mid-slide when neither
        // page covers everything.
        let (backdrop, _) = interactive_pair(ctx.theme, Role::Base200, ctx.state);
        canvas.fill_rect(bounds, backdrop);
        if self.pages.is_empty() {
            return;
        }

        let fraction = self.fraction_at(ctx.now_ms);
        let offset = (i64::from(fraction) * i64::from(bounds.width) / i64::from(WHOLE)) as i32;

        // The current page at its offset, and whichever neighbour the gap
        // exposes — clipped to the bounds, so pages slide behind the
        // rectangle rather than across the panel.
        {
            let mut c = canvas.with_clip(bounds);
            let page_at = |c: &mut Pen<'_>, index: usize, dx: i32| {
                if let Some(page) = self.pages.get(index) {
                    let shifted = Rect::new(bounds.x + dx, bounds.y, bounds.width, bounds.height);
                    page.paint_at(shifted, 0, c);
                }
            };
            page_at(&mut c, self.current, offset);
            if offset > 0 {
                // The current page sits right of rest, so its predecessor
                // shows through on the left.
                page_at(&mut c, self.neighbour(-1), offset - bounds.width);
            } else if offset < 0 {
                page_at(&mut c, self.neighbour(1), offset + bounds.width);
            }
        }

        // The dots: current filled in the role's colour, the others hollow.
        // Display only — a dot is too small a touch target to be honest about.
        if self.pages.len() > 1 {
            let count = self.pages.len() as i32;
            let span = (count - 1) * DOT_GAP;
            let mut x = bounds.x + (bounds.width - span) / 2;
            let y = bounds.bottom() - DOT * 3;
            let (accent, _) = interactive_pair(ctx.theme, self.role, ctx.state);
            let rim = ctx.theme.color(Role::Base100);
            for index in 0..count as usize {
                let centre = Point::new(x, y);
                // A rim under every dot, so they read against any photograph.
                canvas.fill_circle(centre, DOT + 1, rim);
                if index == self.current {
                    canvas.fill_circle(centre, DOT, accent);
                } else {
                    canvas.stroke_circle(centre, DOT, 1, ctx.theme.color(Role::Base300));
                }
                x += DOT_GAP;
            }
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
        if self.pages.len() < 2 {
            return Handled::No;
        }
        let width = ctx.bounds.width.max(1);

        match event {
            Event::Input(InputEvent::PointerButton {
                state: ElementState::Down,
                position,
                ..
            })
            | Event::Input(InputEvent::TouchDown { position, .. }) => {
                if !ctx.bounds.contains(*position) {
                    return Handled::No;
                }
                self.grip = Some((*position, width));
                // A touch catches a slide where it is; the drag takes over
                // from the slide's current displacement.
                self.phase = Phase::Dragging {
                    fraction: self.fraction_at(ctx.now_ms),
                };
                self.held_since = ctx.now_ms;
                Handled::Yes
            }

            Event::Input(InputEvent::PointerMoved { position })
            | Event::Input(InputEvent::TouchMoved { position, .. }) => {
                let Some((grip, width)) = self.grip else {
                    return Handled::No;
                };
                // Clamped to one page: dragging further than the neighbour
                // exposes nothing past it.
                let fraction = ((i64::from(position.x - grip.x) * i64::from(WHOLE))
                    / i64::from(width.max(1))) as i32;
                let fraction = fraction.clamp(-WHOLE, WHOLE);
                if self.phase == (Phase::Dragging { fraction }) {
                    return Handled::No;
                }
                self.phase = Phase::Dragging { fraction };
                Handled::Yes
            }

            Event::Input(InputEvent::PointerButton {
                state: ElementState::Up,
                ..
            })
            | Event::Input(InputEvent::TouchUp { .. }) => {
                if self.grip.take().is_none() {
                    return Handled::No;
                }
                let fraction = match self.phase {
                    Phase::Dragging { fraction } => fraction,
                    _ => 0,
                };
                let commit = WHOLE / COMMIT_DIVISOR;
                if fraction <= -commit {
                    // Dragged left: the next page was pulled in from the right
                    // and now sits a page short of rest.
                    self.arrive(self.neighbour(1), fraction + WHOLE, ctx);
                } else if fraction >= commit {
                    self.arrive(self.neighbour(-1), fraction - WHOLE, ctx);
                } else if fraction != 0 {
                    // Not far enough: spring back. No message — the page did
                    // not change. The hold restarts when the spring lands.
                    self.phase = Phase::Sliding {
                        fraction,
                        from_ms: ctx.now_ms,
                    };
                    ctx.request_animation();
                } else {
                    self.phase = Phase::Still;
                }
                Handled::Yes
            }

            Event::Input(InputEvent::Key {
                code,
                state: ElementState::Down,
                ..
            }) if ctx.state.contains(VisualState::FOCUSED) => match code {
                KeyCode::ArrowRight | KeyCode::ArrowDown => {
                    self.arrive(self.neighbour(1), WHOLE, ctx);
                    Handled::Yes
                }
                KeyCode::ArrowLeft | KeyCode::ArrowUp => {
                    self.arrive(self.neighbour(-1), -WHOLE, ctx);
                    Handled::Yes
                }
                KeyCode::Home if self.current != 0 => {
                    self.arrive(0, -WHOLE, ctx);
                    Handled::Yes
                }
                KeyCode::End if self.current != self.pages.len() - 1 => {
                    self.arrive(self.pages.len() - 1, WHOLE, ctx);
                    Handled::Yes
                }
                KeyCode::Home | KeyCode::End => Handled::Yes,
                _ => Handled::No,
            },
            _ => Handled::No,
        }
    }

    fn animate(&mut self, now_ms: u64) -> Animation {
        match self.phase {
            Phase::Sliding { from_ms, .. } => {
                if now_ms.saturating_sub(from_ms) >= SLIDE_MS {
                    // Arrived. The hold starts now; the advance clock decides
                    // whether there is anything left to wake for.
                    self.phase = Phase::Still;
                    self.held_since = now_ms;
                    Animation {
                        repaint: true,
                        next: self.advance_wake(now_ms),
                    }
                } else {
                    // Mid-slide: the tree's rate, whatever it is set to.
                    Animation::MOVING
                }
            }
            Phase::Dragging { .. } => Animation {
                // A finger holds the pages: nothing moves by itself, and the
                // advance clock waits for it to lift. One distant check keeps
                // the animation alive without costing frames.
                repaint: false,
                next: self.advance_wake(now_ms),
            },
            Phase::Still => match self.advance_ms {
                None => Animation::NONE,
                // A one-page carousel has nothing to advance to, and asking at
                // the interval rather than dropping out keeps the clock running
                // for pages added later.
                Some(_) if self.pages.len() < 2 => Animation {
                    repaint: false,
                    next: self.advance_wake(now_ms),
                },
                Some(interval) => {
                    let due = self.held_since.saturating_add(interval);
                    if now_ms < due {
                        // Holding: one wake at the deadline, the toast
                        // arrangement — no repaint until it fires.
                        Animation::due_at(due)
                    } else {
                        // Due: slide to the next page. No message — see the
                        // note on the type — the clock is the machine talking
                        // to itself.
                        self.current = self.neighbour(1);
                        self.phase = Phase::Sliding {
                            fraction: WHOLE,
                            from_ms: now_ms,
                        };
                        self.held_since = now_ms;
                        Animation::MOVING
                    }
                }
            },
        }
    }

    /// Pages change without sliding, and **keep changing**.
    ///
    /// The distinction the whole [`Motion`](crate::Motion) design turns on: the
    /// slide is motion and goes away, the eight-second advance is a schedule and
    /// does not. A signage rotator under reduced motion is still a rotator — it
    /// cuts between pictures instead of sliding between them.
    fn snap(&mut self, now_ms: u64) -> Animation {
        match self.phase {
            Phase::Sliding { .. } => {
                self.phase = Phase::Still;
                self.held_since = now_ms;
                Animation {
                    repaint: true,
                    next: self.advance_wake(now_ms),
                }
            }
            // A finger is holding the pages where they are. That displacement is
            // not the tree's animation to land — it is where the person put it.
            Phase::Dragging { .. } => Animation {
                repaint: false,
                next: self.advance_wake(now_ms),
            },
            Phase::Still => match self.advance_ms {
                None => Animation::NONE,
                Some(_) if self.pages.len() < 2 => Animation {
                    repaint: false,
                    next: self.advance_wake(now_ms),
                },
                Some(interval) => {
                    let due = self.held_since.saturating_add(interval);
                    if now_ms < due {
                        Animation::due_at(due)
                    } else {
                        self.current = self.neighbour(1);
                        self.held_since = now_ms;
                        Animation {
                            repaint: true,
                            next: self.advance_wake(now_ms),
                        }
                    }
                }
            },
        }
    }

    fn accepts_pointer(&self) -> bool {
        true
    }

    /// One page is nothing to navigate, and a carousel nobody listens to is
    /// display: neither is a tab stop.
    fn focusable(&self) -> bool {
        self.message.is_some() && self.pages.len() > 1
    }
}

impl<M> Describe for Carousel<M> {
    const KIND: &'static str = "carousel";
    const DOC: &'static str = "Pictures shown one at a time, sliding between them.";
    const GROUP: Group = Group::Media;
    const ICON: &'static denise::icon::Icon = &super::icons::CAROUSEL;

    // The pictures are not here. They are child nodes of the form — one
    // `picture` per page, loaded by the engine — because this crate decodes
    // nothing and a property cannot hold a buffer of pixels.
    const PROPERTIES: &'static [Property] = &[
        Property::new(
            "selected",
            PropertyKind::Int {
                min: 0,
                max: i32::MAX,
            },
            "Which page is showing when the form opens; the real upper bound is the number of pictures, which a descriptor cannot see.",
        ),
        Property::new(
            "on-change",
            PropertyKind::Message(Payload::Index),
            "Emitted with the page a person lands on; the advance clock is silent, because a message reports what a person did.",
        ),
        Property::new(
            "auto-advance-ms",
            PropertyKind::Int {
                min: 500,
                max: 60_000,
            },
            "Advance to the next page this often, on the animation clock; without it the carousel only moves when someone moves it.",
        ),
        Property::new(
            "role",
            PropertyKind::Enum(ROLES),
            "Colour of the current page's dot and the focus ring.",
        ),
    ];

    fn get(&self, name: &str) -> Option<Value> {
        Some(match name {
            "selected" => Value::Int(i32::try_from(self.current()).unwrap_or(i32::MAX)),
            // No clock is nothing to report, so a file that never asked for one
            // does not grow one.
            "auto-advance-ms" => Value::Int(i32::try_from(self.advance_ms?).unwrap_or(i32::MAX)),
            "role" => Value::role(self.role),
            // The message is the application's own type; see the `describe`
            // module documentation.
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            // Through the setter, so a page past the end clamps to the last one
            // rather than leaving the carousel pointing at nothing.
            "selected" => self.set_current(value.as_index()?),
            // `auto_advance`'s floor, and it is not cosmetic: an interval the
            // quarter-second slide cannot keep up with is a carousel that never
            // comes to rest.
            "auto-advance-ms" => {
                self.advance_ms = Some(value.as_millis()?.max(SLIDE_MS * 2));
            }
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

    fn picture(word: u32) -> (Vec<u32>, Size) {
        (alloc::vec![word; 16], Size::new(4, 4))
    }

    fn carousel(pages: usize) -> Carousel<usize> {
        let mut c = Carousel::new(|index| index);
        for i in 0..pages {
            let (px, size) = picture(0xFF00_0000 | i as u32);
            c.pages.push(Image::new(px, size).with_fit(Fit::Cover));
        }
        c
    }

    /// A rotator is a cycle: one past the end is the start, in both directions.
    #[test]
    fn the_neighbour_wraps_in_both_directions() {
        let mut c = carousel(3);
        assert_eq!(c.neighbour(1), 1);
        c.set_current(2);
        assert_eq!(c.neighbour(1), 0, "forward past the end wraps");
        c.set_current(0);
        assert_eq!(c.neighbour(-1), 2, "backward past the start wraps");
    }

    /// The slide runs from its displacement to zero, linearly, and is exactly
    /// zero at the end — a slide that lands at 1 leaves a one-pixel seam.
    #[test]
    fn a_slide_runs_to_exactly_rest() {
        assert_eq!(slide_fraction(WHOLE, 0), WHOLE);
        assert_eq!(slide_fraction(WHOLE, SLIDE_MS), 0);
        assert_eq!(slide_fraction(WHOLE, SLIDE_MS * 10), 0, "and stays there");
        assert_eq!(slide_fraction(WHOLE, SLIDE_MS / 2), WHOLE / 2);
        assert_eq!(slide_fraction(-WHOLE, SLIDE_MS / 2), -WHOLE / 2);
        // Monotonic: a slide never moves backwards.
        let mut previous = WHOLE;
        for at in 0..=SLIDE_MS {
            let now = slide_fraction(WHOLE, at);
            assert!(now <= previous, "the slide went backwards at {at}");
            previous = now;
        }
    }

    /// While holding on a page, the animation asks for exactly one wake — the
    /// deadline — and no repaint. This is the cost claim.
    #[test]
    fn holding_asks_for_one_wake_at_the_deadline() {
        let mut c = carousel(3).auto_advance(8_000);
        c.held_since = 1_000;
        let hold = Widget::<usize>::animate(&mut c, 2_000);
        assert!(!hold.repaint, "a hold must not repaint");
        assert_eq!(hold.next, Wake::At(9_000), "one wake, at the deadline");
        // Asked again before the deadline — the tree wakes for the most
        // impatient animation and asks everybody — same answer.
        let again = Widget::<usize>::animate(&mut c, 5_000);
        assert_eq!(again.next, Wake::At(9_000));
        assert_eq!(c.current(), 0, "and the page has not moved");
    }

    /// At the deadline the clock slides to the next page, then rests for a
    /// full interval — and never emits, because no person did anything.
    #[test]
    fn the_advance_clock_slides_and_then_rests() {
        let mut c = carousel(3).auto_advance(8_000);
        c.held_since = 0;
        let due = Widget::<usize>::animate(&mut c, 8_000);
        assert!(due.repaint);
        assert_eq!(due.next, Wake::Animating, "sliding at the tree's rate");
        assert_eq!(c.current(), 1);

        // The slide finishes; the next wake is the next deadline.
        let settled = Widget::<usize>::animate(&mut c, 8_000 + SLIDE_MS);
        assert!(settled.repaint, "the landing frame paints");
        assert_eq!(settled.next, Wake::At(8_000 + SLIDE_MS + 8_000));
        assert_eq!(c.phase, Phase::Still);

        // Asked again mid-hold — the tree wakes for the most impatient
        // animation and asks everybody, so a spinner elsewhere on the screen
        // asks this carousel every frame. The hold must hold: same deadline,
        // no advance. This is what the landing's clock restart is *for*; the
        // landing's own `Wake::At` covers the quiet case by itself.
        let mid_hold = Widget::<usize>::animate(&mut c, 8_000 + SLIDE_MS + 1_000);
        assert!(!mid_hold.repaint);
        assert_eq!(mid_hold.next, Wake::At(8_000 + SLIDE_MS + 8_000));
        assert_eq!(c.current(), 1, "an early ask must not advance the page");
    }

    /// Without an advance clock, a still carousel asks for nothing at all —
    /// the idle-cost floor every widget here is held to.
    #[test]
    fn a_still_carousel_without_a_clock_costs_nothing() {
        let mut c = carousel(3);
        assert_eq!(Widget::<usize>::animate(&mut c, 5_000), Animation::NONE);
    }

    /// A single picture cannot rotate: the clock keeps its distant check but
    /// never slides, and the widget is not a tab stop.
    #[test]
    fn one_page_neither_rotates_nor_takes_focus() {
        let mut c = carousel(1).auto_advance(1_000);
        c.held_since = 0;
        let asked = Widget::<usize>::animate(&mut c, 10_000);
        assert!(!asked.repaint);
        assert_eq!(c.current(), 0, "nowhere to go");
        assert!(!Widget::<usize>::focusable(&c));
        assert!(Widget::<usize>::focusable(&carousel(2)));
        assert!(
            !Widget::<usize>::focusable(&Carousel::<usize>::inert()),
            "a carousel nobody listens to is display"
        );
    }

    /// `set_current` is a silent setter, out of range does nothing, and it
    /// cancels any motion — the application wrote state, the state is shown.
    #[test]
    fn set_current_is_silent_and_clamped() {
        let mut c = carousel(3);
        c.phase = Phase::Sliding {
            fraction: WHOLE,
            from_ms: 0,
        };
        c.set_current(2);
        assert_eq!(c.current(), 2);
        assert_eq!(c.phase, Phase::Still);
        c.set_current(99);
        assert_eq!(c.current(), 2, "out of range does nothing");
    }
}
