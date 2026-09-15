//! Turning raw evdev events into [`InputEvent`]s.
//!
//! This is a state machine, and it is the part of input handling that is actually
//! difficult. It is kept platform-independent so every case below is a unit test
//! rather than something you discover by dragging a finger across a panel in a
//! workshop.
//!
//! Three things it gets right that a naive loop does not:
//!
//! - **Frames.** evdev delivers a position as separate `REL_X` and `REL_Y` events
//!   terminated by `SYN_REPORT`. Emitting on each axis produces two moves per
//!   physical motion, doubling every drag and breaking every gesture threshold.
//!   Nothing is emitted until the frame closes.
//! - **Ordering.** A click arrives in the same frame as the motion that positioned
//!   it. Motion is resolved first, so a button event carries where the pointer
//!   actually was, not where it had been.
//! - **Contact identity.** Multitouch slots are stateful: a slot keeps reporting
//!   for the same finger until its tracking id goes to `-1`, and only the axes
//!   that changed are resent.
//!
//! A touchpad reports the same slots as a touchscreen, and is not one: where the
//! finger is on the pad says nothing about where on the screen it points. A
//! translator told it is reading a touchpad ([`Translator::set_touchpad`]) moves
//! the pointer by how far a finger travels, scrolls with two, and turns a short
//! tap into a click. The rest of the protocol is read the same way.

use core::time::Duration;

use denise::{ElementState, InputEvent, KeyCode, Modifiers, Point, PointerButton, Size, TouchId};

use crate::layout::{self, Composer, Layout};

use crate::codes::{abs, btn, ev, key_value, rel, syn};
use crate::keymap::key_code;

/// Contacts tracked at once. Ten fingers is more than any panel this targets.
pub const MAX_SLOTS: usize = 10;

/// Pixels scrolled per wheel detent.
const SCROLL_STEP: f32 = 40.0;

/// How far a finger's travel across a touchpad moves the pointer: the pad's full
/// width is this many widths of the surface. One pad across for one screen across
/// reaches every point in a stroke without the pointer racing away from a finger
/// that barely moved.
const TOUCHPAD_SPEED: f32 = 1.0;

/// The longest a touch can last and still be a tap.
const TAP_TIME: Duration = Duration::from_millis(200);

/// How far fingers may travel during a tap, as a share of the pad's width. A
/// resting finger drifts; one that moves further than this was pointing.
const TAP_TRAVEL: f32 = 0.03;

/// One event as read from `/dev/input/eventN`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawEvent {
    /// `EV_*` event type.
    pub kind: u16,
    /// Type-specific code.
    pub code: u16,
    /// Type-specific value.
    pub value: i32,
}

impl RawEvent {
    /// Creates a raw event.
    pub const fn new(kind: u16, code: u16, value: i32) -> Self {
        Self { kind, code, value }
    }

    /// The frame separator.
    pub const SYN: Self = Self::new(ev::SYN, syn::REPORT, 0);
}

/// The calibration of one absolute axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbsAxis {
    /// Lowest value the device reports.
    pub min: i32,
    /// Highest value the device reports.
    pub max: i32,
}

impl AbsAxis {
    /// Creates an axis range.
    pub const fn new(min: i32, max: i32) -> Self {
        Self { min, max }
    }

    /// Maps a device reading onto `0..extent` pixels.
    ///
    /// A touchscreen reports in its own units — commonly `0..4095`, which is not
    /// the panel resolution and is not even the same aspect ratio.
    pub fn map(&self, value: i32, extent: u32) -> i32 {
        if extent == 0 {
            return 0;
        }
        let span = i64::from(self.max) - i64::from(self.min);
        if span <= 0 {
            return 0;
        }
        let offset = i64::from(value.clamp(self.min, self.max)) - i64::from(self.min);
        (offset * (i64::from(extent) - 1) / span) as i32
    }
}

/// One multitouch slot.
#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    /// Contact identity from the kernel; `None` when the slot is free.
    tracking_id: Option<i32>,
    x: i32,
    y: i32,
    /// A contact began this frame.
    began: bool,
    /// A position axis changed this frame.
    moved: bool,
    /// The contact ended this frame.
    ended: bool,
}

/// What a touchpad remembers from one frame to the next.
#[derive(Clone, Copy, Debug, Default)]
struct Touchpad {
    /// Where each finger was when the last frame closed, in pad units, by slot.
    /// A single-contact pad keeps its one finger in the first.
    last: [Option<(i32, i32)>; MAX_SLOTS],
    /// Fingers down when the last frame closed.
    fingers: usize,
    /// The part of a pixel travelled but not yet moved, so slow motion is not
    /// lost to rounding.
    carry_x: f32,
    carry_y: f32,
    /// When the fingers now down first touched, if this can still be a tap.
    tap_start: Option<Duration>,
    /// How far they have travelled since, in pad units.
    tap_travel: f32,
    /// The most fingers down at once since, which decides the tap's button.
    tap_fingers: usize,
    /// The pad's own button went down with two fingers on the pad, so its
    /// release is a right button too.
    right_click: bool,
    /// Fingers down as the `BTN_TOOL_*` keys count them, for a pad without
    /// slots, which reports two fingers as one position.
    tool_fingers: usize,
    /// The finger's position on a pad without slots, which resends only the axis
    /// that changed.
    single: (i32, i32),
}

/// A pointer or key event waiting for its frame to close.
#[derive(Clone, Copy, Debug)]
enum Deferred {
    Button {
        button: PointerButton,
        state: ElementState,
    },
    Key {
        code: KeyCode,
        state: ElementState,
        repeat: bool,
    },
}

/// Accumulates raw events and emits [`InputEvent`]s a frame at a time.
#[derive(Clone, Debug)]
pub struct Translator {
    surface: Size,
    pointer: Point,
    modifiers: Modifiers,

    abs_x: Option<AbsAxis>,
    abs_y: Option<AbsAxis>,

    rel_x: i32,
    rel_y: i32,
    scroll_x: f32,
    scroll_y: f32,

    pending_abs_x: Option<i32>,
    pending_abs_y: Option<i32>,

    /// The device has used the slot protocol, so `ABS_X`/`ABS_Y` are not a pointer.
    multitouch: bool,
    slots: [Slot; MAX_SLOTS],
    slot: usize,

    /// Turns key positions into characters. Held here rather than beside the
    /// keymap because composition is a property of the *sequence* of keystrokes,
    /// and this is the only thing that sees them in order.
    composer: Composer,

    /// `BTN_TOUCH` state, for single-contact panels with no slots.
    touch_down: bool,
    single_touch_active: bool,

    deferred: Vec<Deferred>,

    /// Contacts move the pointer rather than touching the surface.
    touchpad: bool,
    pad: Touchpad,
    /// When the event being fed happened, where the caller knows.
    now: Option<Duration>,
}

impl Translator {
    /// Creates a translator for a surface of `size`.
    pub fn new(size: Size) -> Self {
        Self {
            surface: size,
            pointer: Point::new(size.width as i32 / 2, size.height as i32 / 2),
            modifiers: Modifiers::NONE,
            abs_x: None,
            abs_y: None,
            rel_x: 0,
            rel_y: 0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            pending_abs_x: None,
            pending_abs_y: None,
            multitouch: false,
            slots: [Slot::default(); MAX_SLOTS],
            slot: 0,
            touch_down: false,
            single_touch_active: false,
            composer: Composer::new(&layout::US),
            deferred: Vec::new(),
            touchpad: false,
            pad: Touchpad::default(),
            now: None,
        }
    }

    /// Reads this device as a touchpad.
    ///
    /// A finger moves the pointer by how far it travels rather than to where it
    /// is, two fingers scroll, the pad's button is a right button with two
    /// fingers down, and a short tap is a click — a right click with two
    /// fingers. Taps need to know when events happened, so they come only
    /// through [`feed_at`](Self::feed_at).
    pub fn set_touchpad(&mut self, touchpad: bool) {
        self.touchpad = touchpad;
        self.pad = Touchpad::default();
    }

    /// Whether this device is read as a touchpad.
    pub fn is_touchpad(&self) -> bool {
        self.touchpad
    }

    /// Reads this device with a different keyboard layout.
    ///
    /// Defaults to US, because [`KeyCode`] names US positions and defaulting to
    /// anything else would make the two disagree by default.
    pub fn set_layout(&mut self, layout: &'static Layout) {
        self.composer.set_layout(layout);
    }

    /// The layout this device is being read with.
    pub fn layout(&self) -> &'static Layout {
        self.composer.layout()
    }

    /// Declares the range of an absolute axis, from the device's `absinfo`.
    ///
    /// Without this an absolute device is ignored: mapping a reading to a pixel is
    /// impossible without knowing what the reading is out of.
    pub fn set_abs_range(&mut self, axis: u16, range: AbsAxis) {
        match axis {
            abs::X | abs::MT_POSITION_X => self.abs_x = Some(range),
            abs::Y | abs::MT_POSITION_Y => self.abs_y = Some(range),
            _ => {}
        }
    }

    /// Tells the translator the surface changed size.
    pub fn resize(&mut self, size: Size) {
        self.surface = size;
        self.pointer = self.clamp(self.pointer);
    }

    /// Current pointer position.
    pub fn pointer(&self) -> Point {
        self.pointer
    }

    /// Moves the pointer without emitting anything.
    ///
    /// Used to share one cursor between several pointing devices: a mouse and a
    /// tablet should move the same pointer rather than each keeping its own.
    pub fn set_pointer(&mut self, position: Point) {
        self.pointer = self.clamp(position);
    }

    /// Currently held modifiers.
    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// The calibration read from the device, as `(x, y)`.
    pub fn abs_ranges(&self) -> (Option<AbsAxis>, Option<AbsAxis>) {
        (self.abs_x, self.abs_y)
    }

    /// Returns `true` once the device has identified itself as multitouch.
    pub fn is_multitouch(&self) -> bool {
        self.multitouch
    }

    /// Feeds one raw event, appending any completed events to `out`.
    pub fn feed(&mut self, event: RawEvent, out: &mut Vec<InputEvent>) {
        match event.kind {
            ev::SYN => match event.code {
                syn::REPORT => self.flush(out),
                // The kernel's buffer overflowed and events were lost. Anything
                // half-accumulated describes a state that never existed.
                syn::DROPPED => self.discard_frame(),
                _ => {}
            },
            ev::REL => self.feed_rel(event),
            ev::ABS => self.feed_abs(event),
            ev::KEY => self.feed_key(event),
            _ => {}
        }
    }

    /// Feeds one raw event that happened at `at`, on any clock that only moves
    /// forward — the kernel's timestamp, as a duration since the epoch.
    ///
    /// The same as [`feed`](Self::feed), except that a touchpad can tell a tap
    /// from a touch that lasted.
    pub fn feed_at(&mut self, event: RawEvent, at: Duration, out: &mut Vec<InputEvent>) {
        self.now = Some(at);
        self.feed(event, out);
    }

    /// Feeds a whole batch, which need not be frame-aligned.
    pub fn feed_all(&mut self, events: &[RawEvent], out: &mut Vec<InputEvent>) {
        for event in events {
            self.feed(*event, out);
        }
    }

    fn feed_rel(&mut self, event: RawEvent) {
        match event.code {
            rel::X => self.rel_x += event.value,
            rel::Y => self.rel_y += event.value,
            rel::WHEEL => self.scroll_y -= event.value as f32 * SCROLL_STEP,
            rel::HWHEEL => self.scroll_x += event.value as f32 * SCROLL_STEP,
            _ => {}
        }
    }

    fn feed_abs(&mut self, event: RawEvent) {
        match event.code {
            abs::MT_SLOT => {
                self.multitouch = true;
                self.slot = (event.value.max(0) as usize).min(MAX_SLOTS - 1);
            }
            abs::MT_TRACKING_ID => {
                self.multitouch = true;
                let slot = &mut self.slots[self.slot];
                if event.value < 0 {
                    if slot.tracking_id.is_some() {
                        slot.ended = true;
                    }
                } else {
                    slot.tracking_id = Some(event.value);
                    slot.began = true;
                }
            }
            abs::MT_POSITION_X => {
                self.multitouch = true;
                let slot = &mut self.slots[self.slot];
                slot.x = event.value;
                slot.moved = true;
            }
            abs::MT_POSITION_Y => {
                self.multitouch = true;
                let slot = &mut self.slots[self.slot];
                slot.y = event.value;
                slot.moved = true;
            }
            abs::X => self.pending_abs_x = Some(event.value),
            abs::Y => self.pending_abs_y = Some(event.value),
            _ => {}
        }
    }

    fn feed_key(&mut self, event: RawEvent) {
        let state = match event.value {
            key_value::UP => ElementState::Up,
            key_value::DOWN | key_value::REPEAT => ElementState::Down,
            _ => return,
        };
        let repeat = event.value == key_value::REPEAT;

        match event.code {
            btn::LEFT | btn::RIGHT | btn::MIDDLE => {
                // Auto-repeat is meaningless for a button.
                if repeat {
                    return;
                }
                let button = match event.code {
                    btn::LEFT => PointerButton::Left,
                    btn::RIGHT => PointerButton::Right,
                    _ => PointerButton::Middle,
                };
                self.deferred.push(Deferred::Button { button, state });
            }
            btn::TOUCH => self.touch_down = state.is_down(),
            // How many fingers are down, not keys: a pad without slots counts its
            // fingers this way, and on anything else they are not worth a
            // keypress nobody can bind.
            btn::TOOL_FINGER
            | btn::TOOL_DOUBLETAP
            | btn::TOOL_TRIPLETAP
            | btn::TOOL_QUADTAP
            | btn::TOOL_QUINTTAP => {
                if repeat {
                    return;
                }
                let count = match event.code {
                    btn::TOOL_FINGER => 1,
                    btn::TOOL_DOUBLETAP => 2,
                    btn::TOOL_TRIPLETAP => 3,
                    btn::TOOL_QUADTAP => 4,
                    _ => 5,
                };
                if state.is_down() {
                    self.pad.tool_fingers = count;
                } else if self.pad.tool_fingers == count {
                    self.pad.tool_fingers = 0;
                }
            }
            btn::TOOL_PEN => {}
            code => {
                let key = key_code(code);
                self.update_modifiers(key, state.is_down());
                self.deferred.push(Deferred::Key {
                    code: key,
                    state,
                    repeat,
                });
            }
        }
    }

    fn update_modifiers(&mut self, key: KeyCode, down: bool) {
        let bit = match key {
            KeyCode::ShiftLeft | KeyCode::ShiftRight => Modifiers::SHIFT,
            KeyCode::ControlLeft | KeyCode::ControlRight => Modifiers::CTRL,
            KeyCode::AltLeft | KeyCode::AltRight => Modifiers::ALT,
            KeyCode::SuperLeft | KeyCode::SuperRight => Modifiers::SUPER,
            _ => return,
        };
        self.modifiers = self.modifiers.set(bit, down);
    }

    fn clamp(&self, point: Point) -> Point {
        Point::new(
            point
                .x
                .clamp(0, self.surface.width.saturating_sub(1) as i32),
            point
                .y
                .clamp(0, self.surface.height.saturating_sub(1) as i32),
        )
    }

    /// Throws away everything accumulated but not yet emitted.
    fn discard_frame(&mut self) {
        self.rel_x = 0;
        self.rel_y = 0;
        self.scroll_x = 0.0;
        self.scroll_y = 0.0;
        self.pending_abs_x = None;
        self.pending_abs_y = None;
        self.deferred.clear();
        for slot in &mut self.slots {
            slot.began = false;
            slot.moved = false;
            slot.ended = false;
        }
    }

    /// Closes a frame: motion first, then everything positioned by it.
    fn flush(&mut self, out: &mut Vec<InputEvent>) {
        if self.touchpad {
            self.flush_touchpad(out);
        } else if self.multitouch {
            self.flush_slots(out);
        } else {
            self.flush_pointer(out);
        }

        if self.scroll_x != 0.0 || self.scroll_y != 0.0 {
            out.push(InputEvent::PointerScroll {
                delta_x: self.scroll_x,
                delta_y: self.scroll_y,
                position: self.pointer,
            });
            self.scroll_x = 0.0;
            self.scroll_y = 0.0;
        }

        for deferred in std::mem::take(&mut self.deferred) {
            out.push(match deferred {
                Deferred::Button { button, state } => InputEvent::PointerButton {
                    button,
                    state,
                    position: self.pointer,
                    modifiers: self.modifiers,
                },
                Deferred::Key {
                    code,
                    state,
                    repeat,
                } => {
                    // The key event first, then whatever it typed. A binding on
                    // Enter or Escape therefore runs before any text arrives, and
                    // a field can insert every `Text` it sees without filtering.
                    out.push(InputEvent::Key {
                        code,
                        state,
                        repeat,
                        modifiers: self.modifiers,
                    });
                    let composed = self.composer.feed(code, state, self.modifiers);
                    for &ch in composed.as_slice() {
                        out.push(InputEvent::Text { ch });
                    }
                    continue;
                }
            });
        }
    }

    fn flush_pointer(&mut self, out: &mut Vec<InputEvent>) {
        let absolute = self.pending_abs_x.is_some() || self.pending_abs_y.is_some();

        let moved = if absolute {
            let x = match (self.pending_abs_x.take(), self.abs_x) {
                (Some(v), Some(axis)) => axis.map(v, self.surface.width),
                _ => self.pointer.x,
            };
            let y = match (self.pending_abs_y.take(), self.abs_y) {
                (Some(v), Some(axis)) => axis.map(v, self.surface.height),
                _ => self.pointer.y,
            };
            let next = self.clamp(Point::new(x, y));
            let moved = next != self.pointer;
            self.pointer = next;
            moved
        } else if self.rel_x != 0 || self.rel_y != 0 {
            let next = self.clamp(Point::new(
                self.pointer.x + self.rel_x,
                self.pointer.y + self.rel_y,
            ));
            self.rel_x = 0;
            self.rel_y = 0;
            let moved = next != self.pointer;
            self.pointer = next;
            moved
        } else {
            false
        };

        // A single-contact panel: BTN_TOUCH plus plain absolute axes, no slots.
        if self.touch_down || self.single_touch_active {
            self.flush_single_touch(out);
            return;
        }

        if moved {
            out.push(InputEvent::PointerMoved {
                position: self.pointer,
            });
        }
    }

    /// A touchpad's frame: pointer motion or scrolling from how the fingers
    /// travelled, and a tap when they lifted soon enough and had not moved.
    fn flush_touchpad(&mut self, out: &mut Vec<InputEvent>) {
        // Where each finger is now, in pad units.
        let mut now: [Option<(i32, i32)>; MAX_SLOTS] = [None; MAX_SLOTS];
        let fingers = if self.multitouch {
            for (index, slot) in self.slots.iter_mut().enumerate() {
                if slot.tracking_id.is_some() && !slot.ended {
                    now[index] = Some((slot.x, slot.y));
                }
                slot.began = false;
                slot.moved = false;
                if slot.ended {
                    slot.ended = false;
                    slot.tracking_id = None;
                }
            }
            // The legacy single-touch axes repeat the first finger; the slots
            // already said it.
            self.pending_abs_x = None;
            self.pending_abs_y = None;
            now.iter().flatten().count()
        } else {
            if let Some(x) = self.pending_abs_x.take() {
                self.pad.single.0 = x;
            }
            if let Some(y) = self.pending_abs_y.take() {
                self.pad.single.1 = y;
            }
            if self.touch_down {
                now[0] = Some(self.pad.single);
                self.pad.tool_fingers.max(1)
            } else {
                0
            }
        };

        // How far the fingers that were already down travelled, on average. A
        // finger arriving or leaving says nothing about motion, and averaging it
        // in would jump the pointer by the distance between two fingers.
        let (mut dx, mut dy, mut moving) = (0.0f32, 0.0f32, 0u32);
        if fingers == self.pad.fingers {
            for (was, is) in self.pad.last.iter().zip(now.iter()) {
                if let (Some(was), Some(is)) = (was, is) {
                    dx += (is.0 - was.0) as f32;
                    dy += (is.1 - was.1) as f32;
                    moving += 1;
                }
            }
        } else {
            self.pad.carry_x = 0.0;
            self.pad.carry_y = 0.0;
        }
        if moving > 0 {
            dx /= moving as f32;
            dy /= moving as f32;
        }

        let span = self
            .abs_x
            .map_or(0, |axis| i64::from(axis.max) - i64::from(axis.min));
        let scale = if span > 0 {
            self.surface.width as f32 * TOUCHPAD_SPEED / span as f32
        } else {
            1.0
        };

        match fingers {
            1 if moving > 0 => {
                let x = dx * scale + self.pad.carry_x;
                let y = dy * scale + self.pad.carry_y;
                let (step_x, step_y) = (x.trunc(), y.trunc());
                self.pad.carry_x = x - step_x;
                self.pad.carry_y = y - step_y;
                let next = self.clamp(Point::new(
                    self.pointer.x + step_x as i32,
                    self.pointer.y + step_y as i32,
                ));
                if next != self.pointer {
                    self.pointer = next;
                    out.push(InputEvent::PointerMoved {
                        position: self.pointer,
                    });
                }
            }
            // Two fingers or more scroll the way a wheel does: moving them away
            // from you is a detent away from you.
            2.. if moving > 0 && (dx != 0.0 || dy != 0.0) => {
                self.scroll_x += dx * scale;
                self.scroll_y += dy * scale;
            }
            _ => {}
        }

        // Taps. A touch starts one; travel, a press of the pad's own button, or
        // lasting too long ends it; lifting the last finger in time clicks.
        if self.pad.fingers == 0 && fingers > 0 {
            self.pad.tap_start = self.now;
            self.pad.tap_travel = 0.0;
            self.pad.tap_fingers = 0;
        }
        if fingers > 0 {
            self.pad.tap_travel += dx.abs() + dy.abs();
            self.pad.tap_fingers = self.pad.tap_fingers.max(fingers);
            let still = span <= 0 || self.pad.tap_travel <= TAP_TRAVEL * span as f32;
            let brief = match (self.pad.tap_start, self.now) {
                (Some(start), Some(now)) => now.saturating_sub(start) <= TAP_TIME,
                _ => false,
            };
            if !still || !brief {
                self.pad.tap_start = None;
            }
        }

        // The pad's own button: a right button with two fingers down, released
        // as the button it was pressed as.
        for deferred in &mut self.deferred {
            if let Deferred::Button {
                button: button @ PointerButton::Left,
                state,
            } = deferred
            {
                self.pad.tap_start = None;
                match state {
                    ElementState::Down if fingers >= 2 => {
                        self.pad.right_click = true;
                        *button = PointerButton::Right;
                    }
                    ElementState::Up if self.pad.right_click => {
                        self.pad.right_click = false;
                        *button = PointerButton::Right;
                    }
                    _ => {}
                }
            }
        }

        if fingers == 0
            && self.pad.fingers > 0
            && let (Some(start), Some(now)) = (self.pad.tap_start.take(), self.now)
            && now.saturating_sub(start) <= TAP_TIME
        {
            let button = if self.pad.tap_fingers >= 2 {
                PointerButton::Right
            } else {
                PointerButton::Left
            };
            for state in [ElementState::Down, ElementState::Up] {
                self.deferred.push(Deferred::Button { button, state });
            }
        }

        self.pad.last = now;
        self.pad.fingers = fingers;
    }

    fn flush_single_touch(&mut self, out: &mut Vec<InputEvent>) {
        const ID: TouchId = 0;
        match (self.touch_down, self.single_touch_active) {
            (true, false) => {
                self.single_touch_active = true;
                out.push(InputEvent::TouchDown {
                    id: ID,
                    position: self.pointer,
                });
            }
            (true, true) => out.push(InputEvent::TouchMoved {
                id: ID,
                position: self.pointer,
            }),
            (false, true) => {
                self.single_touch_active = false;
                out.push(InputEvent::TouchUp {
                    id: ID,
                    position: self.pointer,
                    cancelled: false,
                });
            }
            (false, false) => {}
        }
    }

    fn flush_slots(&mut self, out: &mut Vec<InputEvent>) {
        for index in 0..MAX_SLOTS {
            let slot = self.slots[index];
            if !(slot.began || slot.moved || slot.ended) {
                continue;
            }

            // The slot index is the contact identity. A slot holds one finger at a
            // time, which is exactly the lifetime TouchId promises, and unlike the
            // kernel's tracking id it cannot wrap around and collide.
            let id = index as TouchId;
            let position = self.map_slot(&slot);

            if slot.began {
                out.push(InputEvent::TouchDown { id, position });
            } else if slot.moved && slot.tracking_id.is_some() {
                out.push(InputEvent::TouchMoved { id, position });
            }

            if slot.ended {
                out.push(InputEvent::TouchUp {
                    id,
                    position,
                    cancelled: false,
                });
            }

            let slot = &mut self.slots[index];
            slot.began = false;
            slot.moved = false;
            if slot.ended {
                slot.ended = false;
                slot.tracking_id = None;
            }
        }
    }

    fn map_slot(&self, slot: &Slot) -> Point {
        let x = self
            .abs_x
            .map_or(slot.x, |axis| axis.map(slot.x, self.surface.width));
        let y = self
            .abs_y
            .map_or(slot.y, |axis| axis.map(slot.y, self.surface.height));
        self.clamp(Point::new(x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SURFACE: Size = Size::new(800, 480);

    fn translator() -> Translator {
        Translator::new(SURFACE)
    }

    fn touchscreen() -> Translator {
        let mut t = translator();
        t.set_abs_range(abs::MT_POSITION_X, AbsAxis::new(0, 4095));
        t.set_abs_range(abs::MT_POSITION_Y, AbsAxis::new(0, 4095));
        t
    }

    fn drain(t: &mut Translator, events: &[RawEvent]) -> Vec<InputEvent> {
        let mut out = Vec::new();
        t.feed_all(events, &mut out);
        out
    }

    #[test]
    fn nothing_is_emitted_before_the_frame_closes() {
        let mut t = translator();
        let out = drain(
            &mut t,
            &[
                RawEvent::new(ev::REL, rel::X, 10),
                RawEvent::new(ev::REL, rel::Y, 5),
            ],
        );
        assert!(out.is_empty(), "emitted mid-frame: {out:?}");
    }

    #[test]
    fn both_axes_become_one_move() {
        // The bug this prevents: one PointerMoved per axis, so every drag travels
        // twice and every gesture threshold trips early.
        let mut t = translator();
        let start = t.pointer();
        let out = drain(
            &mut t,
            &[
                RawEvent::new(ev::REL, rel::X, 10),
                RawEvent::new(ev::REL, rel::Y, 5),
                RawEvent::SYN,
            ],
        );
        assert_eq!(
            out,
            vec![InputEvent::PointerMoved {
                position: Point::new(start.x + 10, start.y + 5)
            }]
        );
    }

    #[test]
    fn relative_motion_is_clamped_to_the_surface() {
        let mut t = translator();
        drain(
            &mut t,
            &[RawEvent::new(ev::REL, rel::X, -100_000), RawEvent::SYN],
        );
        assert_eq!(t.pointer().x, 0);
        drain(
            &mut t,
            &[RawEvent::new(ev::REL, rel::X, 100_000), RawEvent::SYN],
        );
        assert_eq!(t.pointer().x, SURFACE.width as i32 - 1);
    }

    #[test]
    fn a_frame_with_no_motion_emits_nothing() {
        let mut t = translator();
        assert!(drain(&mut t, &[RawEvent::SYN]).is_empty());
    }

    #[test]
    fn absolute_pointers_map_through_their_axis_range() {
        // A QEMU tablet, and every touchscreen: device units, not pixels.
        let mut t = translator();
        t.set_abs_range(abs::X, AbsAxis::new(0, 32767));
        t.set_abs_range(abs::Y, AbsAxis::new(0, 32767));

        let out = drain(
            &mut t,
            &[
                RawEvent::new(ev::ABS, abs::X, 32767),
                RawEvent::new(ev::ABS, abs::Y, 0),
                RawEvent::SYN,
            ],
        );
        assert_eq!(
            out,
            vec![InputEvent::PointerMoved {
                position: Point::new(799, 0)
            }]
        );
    }

    #[test]
    fn an_absolute_axis_with_a_nonzero_minimum_still_reaches_the_origin() {
        let axis = AbsAxis::new(100, 4195);
        assert_eq!(axis.map(100, 800), 0);
        assert_eq!(axis.map(4195, 800), 799);
    }

    #[test]
    fn a_degenerate_axis_does_not_divide_by_zero() {
        assert_eq!(AbsAxis::new(5, 5).map(5, 800), 0);
        assert_eq!(AbsAxis::new(0, 100).map(50, 0), 0);
    }

    #[test]
    fn a_click_carries_the_position_from_its_own_frame() {
        // Motion and button arrive together; resolving the button first would
        // report where the pointer used to be.
        let mut t = translator();
        t.set_abs_range(abs::X, AbsAxis::new(0, 799));
        t.set_abs_range(abs::Y, AbsAxis::new(0, 479));

        // Deliberately away from the initial centre, so the move is a real one.
        let out = drain(
            &mut t,
            &[
                RawEvent::new(ev::ABS, abs::X, 600),
                RawEvent::new(ev::ABS, abs::Y, 300),
                RawEvent::new(ev::KEY, btn::LEFT, key_value::DOWN),
                RawEvent::SYN,
            ],
        );

        assert_eq!(
            out,
            vec![
                InputEvent::PointerMoved {
                    position: Point::new(600, 300)
                },
                InputEvent::PointerButton {
                    button: PointerButton::Left,
                    state: ElementState::Down,
                    position: Point::new(600, 300),
                    modifiers: Modifiers::NONE,
                },
            ]
        );
    }

    #[test]
    fn wheel_detents_become_pixels_and_scroll_the_right_way() {
        let mut t = translator();
        let out = drain(
            &mut t,
            &[RawEvent::new(ev::REL, rel::WHEEL, 1), RawEvent::SYN],
        );
        // One detent away from the user scrolls content up, so delta_y is negative.
        match out.as_slice() {
            [InputEvent::PointerScroll { delta_y, .. }] => assert!(*delta_y < 0.0),
            other => panic!("expected one scroll, got {other:?}"),
        }
    }

    #[test]
    fn modifiers_are_held_across_frames_and_reported_on_keys() {
        let mut t = translator();
        drain(
            &mut t,
            &[RawEvent::new(ev::KEY, 42, key_value::DOWN), RawEvent::SYN],
        );
        assert!(t.modifiers().contains(Modifiers::SHIFT));

        let out = drain(
            &mut t,
            &[RawEvent::new(ev::KEY, 30, key_value::DOWN), RawEvent::SYN],
        );
        assert_eq!(
            out,
            vec![
                InputEvent::Key {
                    code: KeyCode::A,
                    state: ElementState::Down,
                    repeat: false,
                    modifiers: Modifiers::SHIFT,
                },
                // The held shift decides the character as well as the flag.
                InputEvent::Text { ch: 'A' },
            ]
        );

        drain(
            &mut t,
            &[RawEvent::new(ev::KEY, 42, key_value::UP), RawEvent::SYN],
        );
        assert!(t.modifiers().is_empty());
    }

    #[test]
    fn auto_repeat_is_flagged_but_still_a_press() {
        let mut t = translator();
        let out = drain(
            &mut t,
            &[RawEvent::new(ev::KEY, 30, key_value::REPEAT), RawEvent::SYN],
        );
        assert_eq!(
            out,
            vec![
                InputEvent::Key {
                    code: KeyCode::A,
                    state: ElementState::Down,
                    repeat: true,
                    modifiers: Modifiers::NONE,
                },
                // A held key keeps typing, which is the entire point of repeat.
                InputEvent::Text { ch: 'a' },
            ]
        );
    }

    #[test]
    fn raw_evdev_codes_become_norwegian_text() {
        // End to end from the wire: evdev code 39 is the US semicolon position,
        // which on a Norwegian keyboard is where ø lives.
        let mut t = translator();
        t.set_layout(&crate::layout::NORWEGIAN);
        let out = drain(
            &mut t,
            &[RawEvent::new(ev::KEY, 39, key_value::DOWN), RawEvent::SYN],
        );
        assert_eq!(
            out,
            vec![
                InputEvent::Key {
                    code: KeyCode::Semicolon,
                    state: ElementState::Down,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                },
                InputEvent::Text { ch: '\u{00f8}' },
            ]
        );
    }

    #[test]
    fn a_dead_key_spans_two_evdev_frames() {
        // Composition survives the frame boundary, which is the thing a per-frame
        // translator could plausibly get wrong.
        let mut t = translator();
        t.set_layout(&crate::layout::NORWEGIAN);
        let first = drain(
            &mut t,
            &[RawEvent::new(ev::KEY, 27, key_value::DOWN), RawEvent::SYN],
        );
        assert_eq!(
            first.len(),
            1,
            "the dead key reports its position and no text: {first:?}"
        );
        let second = drain(
            &mut t,
            &[RawEvent::new(ev::KEY, 24, key_value::DOWN), RawEvent::SYN],
        );
        assert!(
            second.contains(&InputEvent::Text { ch: '\u{00f6}' }),
            "expected ö from ¨ then o, got {second:?}"
        );
    }

    #[test]
    fn buttons_never_auto_repeat() {
        let mut t = translator();
        let out = drain(
            &mut t,
            &[
                RawEvent::new(ev::KEY, btn::LEFT, key_value::REPEAT),
                RawEvent::SYN,
            ],
        );
        assert!(out.is_empty(), "a held button repeated: {out:?}");
    }

    #[test]
    fn a_single_finger_reports_down_move_and_up() {
        let mut t = touchscreen();
        let down = drain(
            &mut t,
            &[
                RawEvent::new(ev::ABS, abs::MT_SLOT, 0),
                RawEvent::new(ev::ABS, abs::MT_TRACKING_ID, 77),
                RawEvent::new(ev::ABS, abs::MT_POSITION_X, 0),
                RawEvent::new(ev::ABS, abs::MT_POSITION_Y, 0),
                RawEvent::SYN,
            ],
        );
        assert_eq!(
            down,
            vec![InputEvent::TouchDown {
                id: 0,
                position: Point::new(0, 0)
            }]
        );

        // Only the axis that changed is resent, which is the protocol's whole point.
        let moved = drain(
            &mut t,
            &[
                RawEvent::new(ev::ABS, abs::MT_POSITION_X, 4095),
                RawEvent::SYN,
            ],
        );
        assert_eq!(
            moved,
            vec![InputEvent::TouchMoved {
                id: 0,
                position: Point::new(799, 0)
            }]
        );

        let up = drain(
            &mut t,
            &[
                RawEvent::new(ev::ABS, abs::MT_TRACKING_ID, -1),
                RawEvent::SYN,
            ],
        );
        assert_eq!(
            up,
            vec![InputEvent::TouchUp {
                id: 0,
                position: Point::new(799, 0),
                cancelled: false
            }]
        );
    }

    #[test]
    fn two_fingers_keep_separate_identities() {
        let mut t = touchscreen();
        let out = drain(
            &mut t,
            &[
                RawEvent::new(ev::ABS, abs::MT_SLOT, 0),
                RawEvent::new(ev::ABS, abs::MT_TRACKING_ID, 10),
                RawEvent::new(ev::ABS, abs::MT_POSITION_X, 0),
                RawEvent::new(ev::ABS, abs::MT_POSITION_Y, 0),
                RawEvent::new(ev::ABS, abs::MT_SLOT, 1),
                RawEvent::new(ev::ABS, abs::MT_TRACKING_ID, 11),
                RawEvent::new(ev::ABS, abs::MT_POSITION_X, 4095),
                RawEvent::new(ev::ABS, abs::MT_POSITION_Y, 4095),
                RawEvent::SYN,
            ],
        );

        assert_eq!(
            out,
            vec![
                InputEvent::TouchDown {
                    id: 0,
                    position: Point::new(0, 0)
                },
                InputEvent::TouchDown {
                    id: 1,
                    position: Point::new(799, 479)
                },
            ]
        );
    }

    #[test]
    fn lifting_one_finger_leaves_the_other_down() {
        let mut t = touchscreen();
        drain(
            &mut t,
            &[
                RawEvent::new(ev::ABS, abs::MT_SLOT, 0),
                RawEvent::new(ev::ABS, abs::MT_TRACKING_ID, 10),
                RawEvent::new(ev::ABS, abs::MT_SLOT, 1),
                RawEvent::new(ev::ABS, abs::MT_TRACKING_ID, 11),
                RawEvent::SYN,
            ],
        );

        let out = drain(
            &mut t,
            &[
                RawEvent::new(ev::ABS, abs::MT_SLOT, 0),
                RawEvent::new(ev::ABS, abs::MT_TRACKING_ID, -1),
                RawEvent::SYN,
            ],
        );
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], InputEvent::TouchUp { id: 0, .. }));

        // Slot 1 must still be live and still report as itself.
        let out = drain(
            &mut t,
            &[
                RawEvent::new(ev::ABS, abs::MT_SLOT, 1),
                RawEvent::new(ev::ABS, abs::MT_POSITION_X, 2048),
                RawEvent::SYN,
            ],
        );
        assert!(matches!(out[0], InputEvent::TouchMoved { id: 1, .. }));
    }

    #[test]
    fn a_reused_slot_starts_a_new_contact() {
        let mut t = touchscreen();
        drain(
            &mut t,
            &[
                RawEvent::new(ev::ABS, abs::MT_SLOT, 0),
                RawEvent::new(ev::ABS, abs::MT_TRACKING_ID, 10),
                RawEvent::SYN,
                RawEvent::new(ev::ABS, abs::MT_TRACKING_ID, -1),
                RawEvent::SYN,
            ],
        );
        let out = drain(
            &mut t,
            &[
                RawEvent::new(ev::ABS, abs::MT_TRACKING_ID, 12),
                RawEvent::SYN,
            ],
        );
        assert!(
            matches!(out[0], InputEvent::TouchDown { id: 0, .. }),
            "a recycled slot must report a fresh contact, got {out:?}"
        );
    }

    #[test]
    fn a_slot_index_beyond_capacity_does_not_panic() {
        let mut t = touchscreen();
        let out = drain(
            &mut t,
            &[
                RawEvent::new(ev::ABS, abs::MT_SLOT, 9999),
                RawEvent::new(ev::ABS, abs::MT_TRACKING_ID, 1),
                RawEvent::SYN,
            ],
        );
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn single_contact_panels_without_slots_still_report_touches() {
        // BTN_TOUCH plus plain ABS_X/ABS_Y: cheap resistive panels, and the
        // protocol-A fallback.
        let mut t = translator();
        t.set_abs_range(abs::X, AbsAxis::new(0, 4095));
        t.set_abs_range(abs::Y, AbsAxis::new(0, 4095));

        let down = drain(
            &mut t,
            &[
                RawEvent::new(ev::KEY, btn::TOUCH, key_value::DOWN),
                RawEvent::new(ev::ABS, abs::X, 2048),
                RawEvent::new(ev::ABS, abs::Y, 2048),
                RawEvent::SYN,
            ],
        );
        assert!(matches!(down[0], InputEvent::TouchDown { id: 0, .. }));

        let up = drain(
            &mut t,
            &[
                RawEvent::new(ev::KEY, btn::TOUCH, key_value::UP),
                RawEvent::SYN,
            ],
        );
        assert!(matches!(up[0], InputEvent::TouchUp { id: 0, .. }));
    }

    #[test]
    fn a_dropped_frame_is_discarded_rather_than_half_applied() {
        // SYN_DROPPED means the kernel lost events. Whatever accumulated describes
        // a state that never existed, and emitting it desynchronises everything
        // downstream.
        let mut t = translator();
        let out = drain(
            &mut t,
            &[
                RawEvent::new(ev::REL, rel::X, 50),
                RawEvent::new(ev::KEY, btn::LEFT, key_value::DOWN),
                RawEvent::new(ev::SYN, syn::DROPPED, 0),
                RawEvent::SYN,
            ],
        );
        assert!(out.is_empty(), "a dropped frame was emitted: {out:?}");
    }

    #[test]
    fn resizing_pulls_the_pointer_back_inside() {
        let mut t = translator();
        drain(
            &mut t,
            &[RawEvent::new(ev::REL, rel::X, 10_000), RawEvent::SYN],
        );
        assert_eq!(t.pointer().x, 799);

        t.resize(Size::new(320, 240));
        assert_eq!(t.pointer(), Point::new(319, 239));
    }

    #[test]
    fn an_absolute_device_never_drifts_from_repeated_frames() {
        // Absolute readings are positions, not deltas. Re-sending the same
        // reading must not move anything.
        let mut t = translator();
        t.set_abs_range(abs::X, AbsAxis::new(0, 799));
        t.set_abs_range(abs::Y, AbsAxis::new(0, 479));

        drain(
            &mut t,
            &[
                RawEvent::new(ev::ABS, abs::X, 100),
                RawEvent::new(ev::ABS, abs::Y, 100),
                RawEvent::SYN,
            ],
        );
        let again = drain(
            &mut t,
            &[
                RawEvent::new(ev::ABS, abs::X, 100),
                RawEvent::new(ev::ABS, abs::Y, 100),
                RawEvent::SYN,
            ],
        );
        assert!(again.is_empty(), "a repeated position moved the pointer");
        assert_eq!(t.pointer(), Point::new(100, 100));
    }

    // Touchpads. The pad is 1600 by 1000 units on an 800-pixel-wide surface, so
    // a unit of travel is half a pixel.

    fn touchpad() -> Translator {
        let mut t = translator();
        for axis in [abs::X, abs::MT_POSITION_X] {
            t.set_abs_range(axis, AbsAxis::new(0, 1599));
        }
        for axis in [abs::Y, abs::MT_POSITION_Y] {
            t.set_abs_range(axis, AbsAxis::new(0, 999));
        }
        t.set_touchpad(true);
        t
    }

    /// Feeds a frame that happened `ms` milliseconds in.
    fn at(t: &mut Translator, ms: u64, events: &[RawEvent]) -> Vec<InputEvent> {
        let mut out = Vec::new();
        for event in events.iter().chain([&RawEvent::SYN]) {
            t.feed_at(*event, Duration::from_millis(ms), &mut out);
        }
        out
    }

    fn touch(slot: i32, id: i32, x: i32, y: i32) -> [RawEvent; 4] {
        [
            RawEvent::new(ev::ABS, abs::MT_SLOT, slot),
            RawEvent::new(ev::ABS, abs::MT_TRACKING_ID, id),
            RawEvent::new(ev::ABS, abs::MT_POSITION_X, x),
            RawEvent::new(ev::ABS, abs::MT_POSITION_Y, y),
        ]
    }

    fn slide(slot: i32, x: i32, y: i32) -> [RawEvent; 3] {
        [
            RawEvent::new(ev::ABS, abs::MT_SLOT, slot),
            RawEvent::new(ev::ABS, abs::MT_POSITION_X, x),
            RawEvent::new(ev::ABS, abs::MT_POSITION_Y, y),
        ]
    }

    fn lift(slot: i32) -> [RawEvent; 2] {
        [
            RawEvent::new(ev::ABS, abs::MT_SLOT, slot),
            RawEvent::new(ev::ABS, abs::MT_TRACKING_ID, -1),
        ]
    }

    fn buttons(out: &[InputEvent]) -> Vec<(PointerButton, ElementState)> {
        out.iter()
            .filter_map(|event| match event {
                InputEvent::PointerButton { button, state, .. } => Some((*button, *state)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_finger_on_a_touchpad_moves_the_pointer_by_its_travel_not_to_where_it_is() {
        // The bug this fixes: the pad read as a touchscreen, so a finger in its
        // corner touched the corner of the screen and the pointer never moved.
        let mut t = touchpad();
        let start = t.pointer();

        let down = at(&mut t, 0, &touch(0, 1, 0, 0));
        assert!(down.is_empty(), "a finger landing did something: {down:?}");
        assert_eq!(t.pointer(), start);

        let moved = at(&mut t, 10, &slide(0, 200, 100));
        assert_eq!(
            moved,
            vec![InputEvent::PointerMoved {
                position: Point::new(start.x + 100, start.y + 50)
            }]
        );
    }

    #[test]
    fn slow_travel_on_a_touchpad_is_not_lost_to_rounding() {
        let mut t = touchpad();
        let start = t.pointer();
        at(&mut t, 0, &touch(0, 1, 100, 100));
        for step in 1..=10 {
            at(&mut t, step * 10, &slide(0, 100 + step as i32, 100));
        }
        // Ten units at half a pixel each.
        assert_eq!(t.pointer().x, start.x + 5);
    }

    #[test]
    fn two_fingers_scroll_the_way_a_wheel_does_and_leave_the_pointer_alone() {
        let mut t = touchpad();
        let start = t.pointer();
        at(
            &mut t,
            0,
            &[touch(0, 1, 400, 600), touch(1, 2, 800, 600)].concat(),
        );
        // Both fingers move 200 units away from the user: a wheel turned away.
        let out = at(
            &mut t,
            20,
            &[slide(0, 400, 400), slide(1, 800, 400)].concat(),
        );

        assert_eq!(t.pointer(), start);
        assert!(
            !out.iter()
                .any(|e| matches!(e, InputEvent::PointerMoved { .. })),
            "scrolling moved the pointer: {out:?}"
        );
        match out.as_slice() {
            [InputEvent::PointerScroll { delta_y, .. }] => {
                assert!(*delta_y < -90.0 && *delta_y > -110.0, "delta_y {delta_y}");
            }
            other => panic!("expected one scroll, got {other:?}"),
        }
    }

    #[test]
    fn a_second_finger_arriving_does_not_jump_the_pointer() {
        let mut t = touchpad();
        at(&mut t, 0, &touch(0, 1, 100, 100));
        let before = t.pointer();
        let out = at(&mut t, 10, &touch(1, 2, 1500, 900));
        assert!(out.is_empty(), "a finger arriving moved something: {out:?}");
        assert_eq!(t.pointer(), before);
    }

    #[test]
    fn a_quick_still_tap_is_a_left_click_where_the_pointer_is() {
        let mut t = touchpad();
        let pointer = t.pointer();
        at(&mut t, 0, &touch(0, 1, 800, 500));
        let out = at(&mut t, 90, &lift(0));
        assert_eq!(
            out,
            vec![
                InputEvent::PointerButton {
                    button: PointerButton::Left,
                    state: ElementState::Down,
                    position: pointer,
                    modifiers: Modifiers::NONE,
                },
                InputEvent::PointerButton {
                    button: PointerButton::Left,
                    state: ElementState::Up,
                    position: pointer,
                    modifiers: Modifiers::NONE,
                },
            ]
        );
    }

    #[test]
    fn a_two_finger_tap_is_a_right_click() {
        let mut t = touchpad();
        at(&mut t, 0, &touch(0, 1, 600, 500));
        at(&mut t, 15, &touch(1, 2, 900, 500));
        at(&mut t, 80, &lift(1));
        let out = at(&mut t, 95, &lift(0));
        assert_eq!(
            buttons(&out),
            vec![
                (PointerButton::Right, ElementState::Down),
                (PointerButton::Right, ElementState::Up)
            ]
        );
    }

    #[test]
    fn a_touch_that_lasts_is_not_a_tap() {
        let mut t = touchpad();
        at(&mut t, 0, &touch(0, 1, 800, 500));
        let out = at(&mut t, 600, &lift(0));
        assert!(
            buttons(&out).is_empty(),
            "a resting finger clicked: {out:?}"
        );
    }

    #[test]
    fn a_touch_that_travels_is_not_a_tap() {
        let mut t = touchpad();
        at(&mut t, 0, &touch(0, 1, 100, 500));
        at(&mut t, 40, &slide(0, 700, 500));
        let out = at(&mut t, 80, &lift(0));
        assert!(buttons(&out).is_empty(), "pointing clicked: {out:?}");
    }

    #[test]
    fn taps_need_to_know_when_events_happened() {
        // Fed without timestamps there is no telling a tap from a finger left
        // resting, so none is guessed at.
        let mut t = touchpad();
        drain(
            &mut t,
            &[touch(0, 1, 800, 500).as_slice(), &[RawEvent::SYN]].concat(),
        );
        let out = drain(&mut t, &[lift(0).as_slice(), &[RawEvent::SYN]].concat());
        assert!(buttons(&out).is_empty(), "{out:?}");
    }

    #[test]
    fn the_pads_button_with_two_fingers_down_is_a_right_click_released_as_one() {
        let mut t = touchpad();
        at(
            &mut t,
            0,
            &[touch(0, 1, 600, 800), touch(1, 2, 900, 800)].concat(),
        );
        let press = at(
            &mut t,
            300,
            &[RawEvent::new(ev::KEY, btn::LEFT, key_value::DOWN)],
        );
        assert_eq!(
            buttons(&press),
            vec![(PointerButton::Right, ElementState::Down)]
        );

        // One finger lifts before the button comes up: still a right button.
        at(&mut t, 320, &lift(1));
        let release = at(
            &mut t,
            340,
            &[RawEvent::new(ev::KEY, btn::LEFT, key_value::UP)],
        );
        assert_eq!(
            buttons(&release),
            vec![(PointerButton::Right, ElementState::Up)]
        );

        // And lifting the last finger afterwards is not a tap on top of it.
        let lifted = at(&mut t, 360, &lift(0));
        assert!(buttons(&lifted).is_empty(), "{lifted:?}");
    }

    #[test]
    fn the_pads_button_with_one_finger_is_a_left_click() {
        let mut t = touchpad();
        at(&mut t, 0, &touch(0, 1, 600, 800));
        let press = at(
            &mut t,
            300,
            &[RawEvent::new(ev::KEY, btn::LEFT, key_value::DOWN)],
        );
        assert_eq!(
            buttons(&press),
            vec![(PointerButton::Left, ElementState::Down)]
        );
    }

    #[test]
    fn a_touchpad_without_slots_counts_its_fingers_from_its_tools() {
        let mut t = translator();
        t.set_abs_range(abs::X, AbsAxis::new(0, 1599));
        t.set_abs_range(abs::Y, AbsAxis::new(0, 999));
        t.set_touchpad(true);
        let start = t.pointer();

        let down = at(
            &mut t,
            0,
            &[
                RawEvent::new(ev::KEY, btn::TOUCH, key_value::DOWN),
                RawEvent::new(ev::KEY, btn::TOOL_FINGER, key_value::DOWN),
                RawEvent::new(ev::ABS, abs::X, 400),
                RawEvent::new(ev::ABS, abs::Y, 400),
            ],
        );
        assert!(down.is_empty(), "{down:?}");
        let moved = at(&mut t, 10, &[RawEvent::new(ev::ABS, abs::X, 600)]);
        assert_eq!(
            moved,
            vec![InputEvent::PointerMoved {
                position: Point::new(start.x + 100, start.y)
            }]
        );

        // Two fingers, reported as one position and a double-tap tool.
        at(
            &mut t,
            20,
            &[
                RawEvent::new(ev::KEY, btn::TOOL_FINGER, key_value::UP),
                RawEvent::new(ev::KEY, btn::TOOL_DOUBLETAP, key_value::DOWN),
            ],
        );
        let scrolled = at(&mut t, 30, &[RawEvent::new(ev::ABS, abs::Y, 600)]);
        assert!(
            matches!(scrolled.as_slice(), [InputEvent::PointerScroll { delta_y, .. }] if *delta_y > 0.0),
            "{scrolled:?}"
        );
    }

    #[test]
    fn finger_tools_are_not_keys() {
        // Touchscreens report BTN_TOOL_FINGER too; it used to come out as a key
        // press nobody could name.
        let mut t = touchscreen();
        let out = drain(
            &mut t,
            &[
                RawEvent::new(ev::KEY, btn::TOOL_FINGER, key_value::DOWN),
                RawEvent::SYN,
            ],
        );
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn a_touchscreen_still_touches() {
        // The touchpad reading is asked for, never assumed.
        let mut t = touchscreen();
        assert!(!t.is_touchpad());
        let out = drain(
            &mut t,
            &[touch(0, 1, 0, 0).as_slice(), &[RawEvent::SYN]].concat(),
        );
        assert!(
            matches!(out.as_slice(), [InputEvent::TouchDown { id: 0, .. }]),
            "{out:?}"
        );
    }
}
