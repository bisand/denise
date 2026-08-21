//! How fast the tree animates, and how a widget says what it is waiting for.
//!
//! Two different things used to be spelled the same way. A spinner asking for
//! "another frame in 16 ms" and a carousel asking for "the next page in eight
//! seconds" both came back as a millisecond deadline, so nothing could tell a
//! **sample rate** from a **duration** — and a setting that halved one would
//! have silently halved the other. [`Wake`] separates them, and [`Motion`] is
//! the one knob that sets the rate for everything.

/// How often the tree looks at whatever is animating.
///
/// One setting for every moving thing in the tree: spinners, knobs crossing,
/// carousel slides, layout tweens, toast fades. It is a **sample rate**, not a
/// duration — halving it makes animation coarser, never slower. A toggle still
/// crosses in 120 ms and a carousel still advances after eight seconds,
/// whatever this says.
///
/// ```
/// # use denise::{Size, theme};
/// # use denise_ui::{Motion, Ui};
/// # enum Msg { Noop }
/// # let mut ui: Ui<Msg> = Ui::new(Size::new(1920, 1080), theme::DARK);
/// ui.set_motion(Motion::Every(33));  // 30 fps: half the wakes, half the cost
/// ui.set_motion(Motion::None);       // reduced motion, or a tight power budget
/// ```
///
/// # Why this and not a constant per widget
///
/// It used to be a constant per widget — four of them, all saying 16 or 50, all
/// private. That is one decision copied four times and reachable from nowhere,
/// and it is the wrong number in two directions at once: a desktop wants sixty
/// frames a second because a rotating arc at twenty reads as a stutter, and a
/// battery-powered panel wants the arc to cost a third as much. The gallery on a
/// Pi 3A+ is 4.20% of a core at 16 ms and 1.37% at 50, for as long as one
/// spinner is on screen.
///
/// So the widget says *that* it is moving and the tree says *when* to look —
/// which also means a custom widget gets the setting for free, without knowing
/// it exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Motion {
    /// Sample every animation in flight this often, in milliseconds.
    ///
    /// Clamped to at least 1 ms: zero would ask the event loop never to sleep,
    /// which is not a frame rate but a busy loop.
    Every(u64),
    /// Do not animate. Transitions land at their end state immediately, and
    /// nothing in the tree asks to be woken for movement.
    ///
    /// This is the `prefers-reduced-motion` answer, and the right setting on
    /// hardware where any animation is a bad trade. It stops **motion**, not
    /// **schedules**: a tooltip still appears after its dwell, a toast still
    /// goes after its hold, a carousel still advances — those are deadlines,
    /// and a deadline is not a frame rate.
    None,
}

impl Motion {
    /// Sixty frames a second, the default.
    ///
    /// Sixty rather than twenty because a rotating arc is the animation least
    /// forgiving of a low rate: a caret can blink twice a second and a knob can
    /// cross in eight frames, but a ring turning in visible steps reads as a
    /// stutter rather than as a style.
    ///
    /// The spinner said twenty for a while, on the argument that twenty is above
    /// the rate at which a rotation stops reading as separate positions and
    /// costs a third of the wakes. The first half of that turned out to be wrong
    /// by eye: asked for twenty and *given* twenty, the arc is visibly steppy. It
    /// had never actually been tried, because until the desktop backend started
    /// honouring `next_wake_ms` the loop free-ran at 60 Hz and quietly delivered
    /// sixty.
    ///
    /// The second half was right, and is why sixty is affordable: the drawing is
    /// not the expense — the #17 bench puts a spinner-sized arc at about three
    /// microseconds — the **wake** is, and each wake ends in a present. That was
    /// costing 16 MB of copying on macOS until `denise-winit` started handing the
    /// compositor an `IOSurface`; a present there is now free, a DRM page flip
    /// always was, and win32 blits the damage rectangle.
    ///
    /// Which is also why it is a default and not a constant. Sixty wakes a
    /// second for a widget that can keep a device awake indefinitely is a small
    /// cost on a desktop and a real one on a battery.
    pub const DEFAULT_INTERVAL_MS: u64 = 16;

    /// The sampling interval in milliseconds, or `None` under [`Motion::None`].
    #[inline]
    pub const fn interval_ms(self) -> Option<u64> {
        match self {
            // `max(1)` rather than a rejected value: a caller asking for zero
            // wants "as fast as possible", and the fastest this can honestly
            // promise is one millisecond.
            Self::Every(ms) => Some(if ms == 0 { 1 } else { ms }),
            Self::None => None,
        }
    }

    /// Whether anything is allowed to move.
    #[inline]
    pub const fn animates(self) -> bool {
        matches!(self, Self::Every(_))
    }
}

impl Default for Motion {
    /// [`Motion::Every`] at [`Motion::DEFAULT_INTERVAL_MS`].
    fn default() -> Self {
        Self::Every(Self::DEFAULT_INTERVAL_MS)
    }
}

/// When a widget wants [`Widget::animate`](crate::Widget::animate) called again.
///
/// The distinction this type exists for:
///
/// - [`Wake::Animating`] is a **rate**. The widget is mid-movement and wants to
///   be looked at as often as the tree looks at movement — so [`Motion`] decides
///   how often, and turning it down costs the animation resolution and nothing
///   else.
/// - [`Wake::At`] is a **deadline**. Something happens at that reading of the
///   clock: a carousel advances, a caret flips, a toast expires. [`Motion`] does
///   not touch it, because quantising a schedule to a frame rate would be a bug.
///
/// Both were `Option<u64>` before, and the difference was invisible.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Wake {
    /// Nothing more to do. The widget drops out of the animating set, which is
    /// the only way out of it — see [`Widget::animate`](crate::Widget::animate).
    #[default]
    Never,
    /// Again at the tree's animation rate, because this widget is moving.
    Animating,
    /// At this reading of the application's clock, whatever the rate is.
    At(u64),
}
