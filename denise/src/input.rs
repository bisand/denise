//! Backend-independent input events.
//!
//! Coordinates are physical pixels relative to the surface origin, already scaled.
//! Widgets should never need to know the scale factor to hit-test.

use alloc::vec::Vec;

use crate::geom::{Point, Size};

/// Identifies one finger within a touch sequence.
pub type TouchId = u64;

/// Whether a button or key went down or came up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ElementState {
    /// Pressed.
    Down,
    /// Released.
    Up,
}

impl ElementState {
    /// Returns `true` for [`ElementState::Down`].
    #[inline]
    pub const fn is_down(self) -> bool {
        matches!(self, ElementState::Down)
    }
}

/// A pointer button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PointerButton {
    /// Primary button.
    Left,
    /// Secondary button.
    Right,
    /// Wheel button.
    Middle,
    /// Any other button, by platform index.
    Other(u16),
}

/// Held modifier keys.
///
/// A hand-rolled bitset rather than a `bitflags` dependency; the core has few
/// enough of these to not warrant one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers(u8);

impl Modifiers {
    /// Nothing held.
    pub const NONE: Self = Self(0);
    /// Either shift key.
    pub const SHIFT: Self = Self(1 << 0);
    /// Either control key.
    pub const CTRL: Self = Self(1 << 1);
    /// Either alt key (`AltGr` reports as `ALT | CTRL` on some platforms).
    pub const ALT: Self = Self(1 << 2);
    /// Either super/meta/command key.
    pub const SUPER: Self = Self(1 << 3);

    /// Returns `true` if every bit in `other` is held.
    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Sets or clears `other`.
    #[inline]
    pub const fn set(self, other: Self, on: bool) -> Self {
        Self(if on {
            self.0 | other.0
        } else {
            self.0 & !other.0
        })
    }

    /// Returns `true` if no modifier is held.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl core::ops::BitOr for Modifiers {
    type Output = Self;
    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for Modifiers {
    #[inline]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// A physical key position, named after the US layout as a convention.
///
/// This is a *position*, not a character. `KeyCode::Semicolon` is where `ø` lives
/// on a Norwegian keyboard; text arrives separately as [`InputEvent::Text`], which
/// is also how dead-key composition and IME output get through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[allow(missing_docs)]
pub enum KeyCode {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,

    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,

    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,

    Escape,
    Enter,
    Tab,
    Space,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,

    ShiftLeft,
    ShiftRight,
    ControlLeft,
    ControlRight,
    AltLeft,
    AltRight,
    SuperLeft,
    SuperRight,
    CapsLock,
    NumLock,
    ScrollLock,

    Minus,
    Equal,
    BracketLeft,
    BracketRight,
    Backslash,
    Semicolon,
    Quote,
    Backquote,
    Comma,
    Period,
    Slash,

    NumpadEnter,
    NumpadAdd,
    NumpadSubtract,
    NumpadMultiply,
    NumpadDivide,
    NumpadDecimal,
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9,

    /// A key this build does not name. Carries the raw platform scancode where one
    /// is available, `0` otherwise. evdev codes survive here.
    Unidentified(u32),
}

/// Something the user or the windowing system did.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum InputEvent {
    /// The pointer moved to `position`.
    PointerMoved {
        /// New position, in physical pixels.
        position: Point,
    },
    /// A pointer button changed state.
    PointerButton {
        /// Which button.
        button: PointerButton,
        /// Down or up.
        state: ElementState,
        /// Where it happened.
        position: Point,
        /// Modifiers held at the time.
        modifiers: Modifiers,
    },
    /// The wheel or a scroll gesture moved. Positive `y` scrolls content down.
    PointerScroll {
        /// Horizontal delta in physical pixels.
        delta_x: f32,
        /// Vertical delta in physical pixels.
        delta_y: f32,
        /// Pointer position at the time.
        position: Point,
    },
    /// The pointer left the surface. Clear hover state.
    PointerLeft,

    /// A finger touched down.
    TouchDown {
        /// Finger identity, valid until the matching [`InputEvent::TouchUp`].
        id: TouchId,
        /// Contact position.
        position: Point,
    },
    /// A finger moved while down.
    TouchMoved {
        /// Finger identity.
        id: TouchId,
        /// New contact position.
        position: Point,
    },
    /// A finger lifted, or its sequence was cancelled.
    TouchUp {
        /// Finger identity.
        id: TouchId,
        /// Last known position.
        position: Point,
        /// `true` if the system cancelled the sequence rather than the user lifting.
        cancelled: bool,
    },

    /// A key changed state.
    Key {
        /// Physical key position.
        code: KeyCode,
        /// Down or up.
        state: ElementState,
        /// `true` for auto-repeat.
        repeat: bool,
        /// Modifiers held at the time.
        modifiers: Modifiers,
    },
    /// Committed text, one character at a time.
    ///
    /// Emitted after dead-key composition, so pressing `¨` then `o` yields exactly
    /// one `Text('ö')` and no text for the dead key itself.
    Text {
        /// The character.
        ch: char,
    },

    /// The surface changed size or DPI. Invalidate layout and damage history.
    SurfaceResized {
        /// New extent in physical pixels.
        size: Size,
        /// New physical-per-logical pixel ratio.
        scale_factor: f32,
    },
    /// The user asked to close. The application decides whether to honour it.
    CloseRequested,
}

/// A source of platform input.
///
/// `poll` is non-blocking and drains whatever is queued. On Linux the same object
/// will also expose its file descriptors so an event loop can `epoll` on input and
/// vblank together rather than spinning — that arrives with `denise-evdev` in M2.
pub trait InputSource {
    /// Appends every pending event to `out`, leaving existing contents alone.
    fn poll(&mut self, out: &mut Vec<InputEvent>);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_bits() {
        let m = Modifiers::NONE | Modifiers::SHIFT | Modifiers::CTRL;
        assert!(m.contains(Modifiers::SHIFT));
        assert!(m.contains(Modifiers::SHIFT | Modifiers::CTRL));
        assert!(!m.contains(Modifiers::ALT));
        assert!(!m.is_empty());
        assert!(m.set(Modifiers::SHIFT, false).contains(Modifiers::CTRL));
        assert!(!m.set(Modifiers::SHIFT, false).contains(Modifiers::SHIFT));
    }

    #[test]
    fn unidentified_keys_keep_their_scancode() {
        assert_ne!(KeyCode::Unidentified(30), KeyCode::Unidentified(31));
    }
}
