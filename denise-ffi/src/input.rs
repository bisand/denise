//! Feeding host input into the tree.
//!
//! One call per event rather than an array of a tagged union. Every host this
//! targets already has its events one at a time — `WM_LBUTTONDOWN`,
//! `-[NSView mouseDown:]`, a `Control.KeyPress` handler — so batching them would
//! mean the host building an array to hand to a library that immediately walks it
//! again.
//!
//! Text arrives separately from keys, and both are needed. A key event is a
//! *position* and drives navigation and shortcuts; text is what a dead key, a
//! compose sequence or an IME finally committed. A host that sends only keys
//! cannot type `ø`, and one that sends only text cannot press Tab.

use denise::{ElementState, InputEvent, Point};

use crate::keys;
use crate::types::{button, modifiers};
use crate::{
    DENISE_ERR_INVALID, DENISE_ERR_NULL, DENISE_ERR_PANIC, DENISE_OK, DeniseUi, guard, handle,
};

/// Delivers one event, resolving the handle first.
///
/// # Safety
///
/// `ui` must be `NULL` or a live handle.
unsafe fn send(ui: *mut DeniseUi, event: InputEvent) -> i32 {
    guard(DENISE_ERR_PANIC, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        let Some(handle) = (unsafe { handle(ui) }) else {
            return DENISE_ERR_NULL;
        };
        handle.ui.handle(&[event]);
        DENISE_OK
    })
}

/// The pointer moved to a position in surface pixels.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_pointer_moved(ui: *mut DeniseUi, x: i32, y: i32) -> i32 {
    // SAFETY: forwarding the caller's promise about `ui`.
    unsafe {
        send(
            ui,
            InputEvent::PointerMoved {
                position: Point::new(x, y),
            },
        )
    }
}

/// A pointer button went down or came up.
///
/// `button` is [`DENISE_BUTTON_LEFT`](crate::DENISE_BUTTON_LEFT) or one of its
/// neighbours; `modifiers` is a bitset of `DENISE_MOD_*`.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_pointer_button(
    ui: *mut DeniseUi,
    button_value: u32,
    down: bool,
    x: i32,
    y: i32,
    modifier_bits: u32,
) -> i32 {
    let Some(button) = button(button_value) else {
        return DENISE_ERR_INVALID;
    };
    // SAFETY: forwarding the caller's promise about `ui`.
    unsafe {
        send(
            ui,
            InputEvent::PointerButton {
                button,
                state: state(down),
                position: Point::new(x, y),
                modifiers: modifiers(modifier_bits),
            },
        )
    }
}

/// The wheel or a scroll gesture moved, in surface pixels. Positive `delta_y`
/// scrolls content down.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_pointer_scroll(
    ui: *mut DeniseUi,
    delta_x: f32,
    delta_y: f32,
    x: i32,
    y: i32,
) -> i32 {
    // SAFETY: forwarding the caller's promise about `ui`.
    unsafe {
        send(
            ui,
            InputEvent::PointerScroll {
                delta_x,
                delta_y,
                position: Point::new(x, y),
            },
        )
    }
}

/// The pointer left the surface. Clears hover state.
///
/// Worth sending. Without it, a host whose window loses the pointer leaves a
/// button lit under a cursor that is somewhere else entirely.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_pointer_left(ui: *mut DeniseUi) -> i32 {
    // SAFETY: forwarding the caller's promise about `ui`.
    unsafe { send(ui, InputEvent::PointerLeft) }
}

/// A finger touched down, moved, or lifted.
///
/// `id` identifies one finger for as long as it is down, and `phase` is 0 for
/// down, 1 for moved, 2 for up and 3 for cancelled — cancelled being the system
/// taking the sequence away, which is not the same as the user lifting.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_touch(
    ui: *mut DeniseUi,
    id: u64,
    phase: u32,
    x: i32,
    y: i32,
) -> i32 {
    let position = Point::new(x, y);
    let event = match phase {
        DENISE_TOUCH_DOWN => InputEvent::TouchDown { id, position },
        DENISE_TOUCH_MOVED => InputEvent::TouchMoved { id, position },
        DENISE_TOUCH_UP => InputEvent::TouchUp {
            id,
            position,
            cancelled: false,
        },
        DENISE_TOUCH_CANCELLED => InputEvent::TouchUp {
            id,
            position,
            cancelled: true,
        },
        _ => return DENISE_ERR_INVALID,
    };
    // SAFETY: forwarding the caller's promise about `ui`.
    unsafe { send(ui, event) }
}

/// A finger made contact.
pub const DENISE_TOUCH_DOWN: u32 = 0;
/// A finger moved while down.
pub const DENISE_TOUCH_MOVED: u32 = 1;
/// A finger lifted.
pub const DENISE_TOUCH_UP: u32 = 2;
/// The system took the sequence away.
pub const DENISE_TOUCH_CANCELLED: u32 = 3;

/// A key position went down or came up.
///
/// `key` is one of the `DENISE_KEY_*` numbers. This is a *position*, not a
/// character: send `denise_ui_text` for what was typed.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_key(
    ui: *mut DeniseUi,
    key: u32,
    down: bool,
    repeat: bool,
    modifier_bits: u32,
) -> i32 {
    let Some(code) = keys::from_abi(key) else {
        return DENISE_ERR_INVALID;
    };
    // SAFETY: forwarding the caller's promise about `ui`.
    unsafe {
        send(
            ui,
            InputEvent::Key {
                code,
                state: state(down),
                repeat,
                modifiers: modifiers(modifier_bits),
            },
        )
    }
}

/// One committed character, as a Unicode scalar value.
///
/// Send this *after* the key that produced it, and send nothing for a dead key —
/// `¨` then `o` is one call, with `U+00F6`.
///
/// Control characters are rejected with [`DENISE_ERR_INVALID`]: Enter, Tab and
/// Backspace are keys, and a host that sends them as text as well makes a field
/// insert a `\r` it can never show.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_text(ui: *mut DeniseUi, codepoint: u32) -> i32 {
    let Some(ch) = char::from_u32(codepoint) else {
        return DENISE_ERR_INVALID;
    };
    if ch.is_control() {
        return DENISE_ERR_INVALID;
    }
    // SAFETY: forwarding the caller's promise about `ui`.
    unsafe { send(ui, InputEvent::Text { ch }) }
}

/// Advances the clock, which is what drives the caret blink and any animation.
///
/// `now_ms` is a monotonic millisecond count whose origin does not matter, only
/// that it never goes backwards. Call it once a frame, before painting.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_tick(ui: *mut DeniseUi, now_ms: u64) -> i32 {
    guard(DENISE_ERR_PANIC, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        let Some(handle) = (unsafe { handle(ui) }) else {
            return DENISE_ERR_NULL;
        };
        handle.ui.tick(now_ms);
        DENISE_OK
    })
}

/// When the next [`denise_ui_tick`] is due, on the same clock, or `-1` if nothing
/// is animating.
///
/// This is what lets a host block instead of poll. On Win32 it is a `SetTimer`
/// interval; on a bare frame loop it is the `poll` timeout. Without it the choice
/// is a spinning idle loop or a caret that does not blink.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_next_wake_ms(ui: *mut DeniseUi) -> i64 {
    guard(-1, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        let Some(handle) = (unsafe { handle(ui) }) else {
            return -1;
        };
        handle.ui.next_wake_ms().map_or(-1, |ms| ms as i64)
    })
}

/// Shows or hides the composited cursor sprite, and stops Denise deciding.
///
/// Left alone, the sprite appears on the first pointer motion and disappears when
/// a finger arrives — right for a panel with no window system underneath. An
/// embedded host is the other case: it already has a system cursor, and a second
/// one a frame behind it is worse than none. Call this once with `false` at
/// startup and it stays off for good.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_show_cursor(ui: *mut DeniseUi, visible: bool) -> i32 {
    guard(DENISE_ERR_PANIC, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        let Some(handle) = (unsafe { handle(ui) }) else {
            return DENISE_ERR_NULL;
        };
        handle.ui.show_cursor(visible);
        DENISE_OK
    })
}

fn state(down: bool) -> ElementState {
    if down {
        ElementState::Down
    } else {
        ElementState::Up
    }
}
