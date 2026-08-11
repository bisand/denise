//! Win32 virtual key codes to [`KeyCode`].
//!
//! A `VK_` code is *almost* a position. The letters and digits are the US ASCII
//! values by definition, and `VK_OEM_1` through `VK_OEM_8` are positions named
//! after where they sit on a US keyboard — which is exactly what [`KeyCode`]
//! means, so this is mostly a rename.
//!
//! Characters come separately, from `WM_CHAR`, after Windows has run the layout,
//! any dead key and any IME. A build that mapped `VK_OEM_1` to `;` could not type
//! `ø`, which is what lives on that position in Norway.

use denise::KeyCode;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

/// The key position for a virtual key code, or `Unidentified` carrying the raw
/// value.
///
/// `extended` is bit 24 of the `WM_KEYDOWN` `lParam`, and it is the only thing
/// that separates the numpad Enter from the main one, or right control from left.
/// Windows sends the same `VK_` code for both.
pub fn key_code(virtual_key: u16, extended: bool) -> KeyCode {
    match VIRTUAL_KEY(virtual_key) {
        // The letters and digits are their ASCII values, which the API guarantees
        // rather than merely happens to do.
        VIRTUAL_KEY(vk @ 0x41..=0x5A) => LETTERS[(vk - 0x41) as usize],
        VIRTUAL_KEY(vk @ 0x30..=0x39) => DIGITS[(vk - 0x30) as usize],

        VK_OEM_MINUS => KeyCode::Minus,
        VK_OEM_PLUS => KeyCode::Equal,
        VK_OEM_4 => KeyCode::BracketLeft,
        VK_OEM_6 => KeyCode::BracketRight,
        VK_OEM_5 => KeyCode::Backslash,
        VK_OEM_1 => KeyCode::Semicolon,
        VK_OEM_7 => KeyCode::Quote,
        VK_OEM_3 => KeyCode::Backquote,
        VK_OEM_COMMA => KeyCode::Comma,
        VK_OEM_PERIOD => KeyCode::Period,
        VK_OEM_2 => KeyCode::Slash,
        // `VK_OEM_102`: the extra key an ISO keyboard has between left shift and
        // `Z`, carrying `<`, `>` and `\` on a Norwegian layout. A build that never
        // names it cannot type a backslash.
        VK_OEM_102 => KeyCode::IntlBackslash,

        VK_SPACE => KeyCode::Space,
        VK_RETURN if extended => KeyCode::NumpadEnter,
        VK_RETURN => KeyCode::Enter,
        VK_TAB => KeyCode::Tab,
        VK_BACK => KeyCode::Backspace,
        VK_ESCAPE => KeyCode::Escape,
        VK_DELETE => KeyCode::Delete,
        VK_INSERT => KeyCode::Insert,
        VK_HOME => KeyCode::Home,
        VK_END => KeyCode::End,
        VK_PRIOR => KeyCode::PageUp,
        VK_NEXT => KeyCode::PageDown,
        VK_UP => KeyCode::ArrowUp,
        VK_DOWN => KeyCode::ArrowDown,
        VK_LEFT => KeyCode::ArrowLeft,
        VK_RIGHT => KeyCode::ArrowRight,

        VK_LSHIFT => KeyCode::ShiftLeft,
        VK_RSHIFT => KeyCode::ShiftRight,
        VK_LCONTROL => KeyCode::ControlLeft,
        VK_RCONTROL => KeyCode::ControlRight,
        VK_LMENU => KeyCode::AltLeft,
        VK_RMENU => KeyCode::AltRight,
        VK_LWIN => KeyCode::SuperLeft,
        VK_RWIN => KeyCode::SuperRight,
        // The unsided codes arrive when nothing has told Windows to distinguish
        // them. The extended bit does, for control and alt: AltGr is reported as
        // extended right alt, and losing that would make it indistinguishable from
        // a plain alt — which is a shortcut, not a third level.
        VK_SHIFT => KeyCode::ShiftLeft,
        VK_CONTROL if extended => KeyCode::ControlRight,
        VK_CONTROL => KeyCode::ControlLeft,
        VK_MENU if extended => KeyCode::AltRight,
        VK_MENU => KeyCode::AltLeft,
        VK_CAPITAL => KeyCode::CapsLock,
        VK_NUMLOCK => KeyCode::NumLock,
        VK_SCROLL => KeyCode::ScrollLock,

        VK_F1 => KeyCode::F1,
        VK_F2 => KeyCode::F2,
        VK_F3 => KeyCode::F3,
        VK_F4 => KeyCode::F4,
        VK_F5 => KeyCode::F5,
        VK_F6 => KeyCode::F6,
        VK_F7 => KeyCode::F7,
        VK_F8 => KeyCode::F8,
        VK_F9 => KeyCode::F9,
        VK_F10 => KeyCode::F10,
        VK_F11 => KeyCode::F11,
        VK_F12 => KeyCode::F12,

        VK_NUMPAD0 => KeyCode::Numpad0,
        VK_NUMPAD1 => KeyCode::Numpad1,
        VK_NUMPAD2 => KeyCode::Numpad2,
        VK_NUMPAD3 => KeyCode::Numpad3,
        VK_NUMPAD4 => KeyCode::Numpad4,
        VK_NUMPAD5 => KeyCode::Numpad5,
        VK_NUMPAD6 => KeyCode::Numpad6,
        VK_NUMPAD7 => KeyCode::Numpad7,
        VK_NUMPAD8 => KeyCode::Numpad8,
        VK_NUMPAD9 => KeyCode::Numpad9,
        VK_ADD => KeyCode::NumpadAdd,
        VK_SUBTRACT => KeyCode::NumpadSubtract,
        VK_MULTIPLY => KeyCode::NumpadMultiply,
        VK_DIVIDE => KeyCode::NumpadDivide,
        VK_DECIMAL => KeyCode::NumpadDecimal,

        _ => KeyCode::Unidentified(virtual_key as u32),
    }
}

const LETTERS: [KeyCode; 26] = [
    KeyCode::A,
    KeyCode::B,
    KeyCode::C,
    KeyCode::D,
    KeyCode::E,
    KeyCode::F,
    KeyCode::G,
    KeyCode::H,
    KeyCode::I,
    KeyCode::J,
    KeyCode::K,
    KeyCode::L,
    KeyCode::M,
    KeyCode::N,
    KeyCode::O,
    KeyCode::P,
    KeyCode::Q,
    KeyCode::R,
    KeyCode::S,
    KeyCode::T,
    KeyCode::U,
    KeyCode::V,
    KeyCode::W,
    KeyCode::X,
    KeyCode::Y,
    KeyCode::Z,
];

const DIGITS: [KeyCode; 10] = [
    KeyCode::Digit0,
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_and_digits_are_their_ascii_values() {
        assert_eq!(key_code(b'A' as u16, false), KeyCode::A);
        assert_eq!(key_code(b'Z' as u16, false), KeyCode::Z);
        assert_eq!(key_code(b'0' as u16, false), KeyCode::Digit0);
        assert_eq!(key_code(b'9' as u16, false), KeyCode::Digit9);
    }

    /// The extended bit is the only thing separating several pairs. Losing it
    /// makes AltGr look like Alt, which turns a third keyboard level into a
    /// shortcut modifier — the exact bug M4 found on Linux, arriving by a
    /// different route.
    #[test]
    fn the_extended_bit_separates_the_pairs_it_has_to() {
        assert_eq!(key_code(VK_MENU.0, false), KeyCode::AltLeft);
        assert_eq!(key_code(VK_MENU.0, true), KeyCode::AltRight);
        assert_eq!(key_code(VK_CONTROL.0, false), KeyCode::ControlLeft);
        assert_eq!(key_code(VK_CONTROL.0, true), KeyCode::ControlRight);
        assert_eq!(key_code(VK_RETURN.0, false), KeyCode::Enter);
        assert_eq!(key_code(VK_RETURN.0, true), KeyCode::NumpadEnter);
    }

    #[test]
    fn the_iso_key_is_named() {
        assert_eq!(key_code(VK_OEM_102.0, false), KeyCode::IntlBackslash);
    }

    /// Two positions naming the same key would make one untypeable and the other
    /// ambiguous, and neither shows up as an error anywhere.
    #[test]
    fn no_two_virtual_keys_name_the_same_position() {
        let mut seen: Vec<(u16, KeyCode)> = Vec::new();
        for virtual_key in 0u16..=0xFF {
            let code = key_code(virtual_key, false);
            if matches!(code, KeyCode::Unidentified(_)) {
                continue;
            }
            // The unsided VK_SHIFT deliberately shares with VK_LSHIFT: Windows
            // sends it when nothing has asked it to distinguish the two, and
            // reporting "some shift" as the left one is the useful answer.
            if code == KeyCode::ShiftLeft {
                continue;
            }
            if let Some((other, _)) = seen.iter().find(|(_, c)| *c == code) {
                panic!("{virtual_key:#04X} and {other:#04X} both name {code:?}");
            }
            seen.push((virtual_key, code));
        }
        assert!(seen.len() > 90, "the table lost entries: {}", seen.len());
    }

    #[test]
    fn an_unknown_code_keeps_its_number() {
        assert_eq!(key_code(0xFE, false), KeyCode::Unidentified(0xFE));
    }
}
