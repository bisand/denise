//! `NSEvent` virtual key codes to [`KeyCode`].
//!
//! These are the ANSI virtual key codes from `<Carbon/HIToolbox/Events.h>`, which
//! are *positions* on a US keyboard and have been since 1984 — which is exactly
//! what [`KeyCode`] means, so this is a rename rather than a translation.
//!
//! The characters are a separate matter. AppKit hands those over as
//! `[NSEvent characters]`, already through the user's layout and already through
//! any dead key or IME, and they arrive as [`InputEvent::Text`](denise::InputEvent).
//! A build that mapped `kVK_ANSI_Semicolon` to `;` would be unable to type `ø`.

use denise::KeyCode;

/// The key position for an `NSEvent` `keyCode`, or `Unidentified` carrying the
/// raw code.
///
/// Nothing here can fail: an unknown code is still a key the user pressed, and a
/// host that can distinguish two of them by their raw value is better off than
/// one that received nothing.
pub fn key_code(virtual_key: u16) -> KeyCode {
    match virtual_key {
        0x00 => KeyCode::A,
        0x0B => KeyCode::B,
        0x08 => KeyCode::C,
        0x02 => KeyCode::D,
        0x0E => KeyCode::E,
        0x03 => KeyCode::F,
        0x05 => KeyCode::G,
        0x04 => KeyCode::H,
        0x22 => KeyCode::I,
        0x26 => KeyCode::J,
        0x28 => KeyCode::K,
        0x25 => KeyCode::L,
        0x2E => KeyCode::M,
        0x2D => KeyCode::N,
        0x1F => KeyCode::O,
        0x23 => KeyCode::P,
        0x0C => KeyCode::Q,
        0x0F => KeyCode::R,
        0x01 => KeyCode::S,
        0x11 => KeyCode::T,
        0x20 => KeyCode::U,
        0x09 => KeyCode::V,
        0x0D => KeyCode::W,
        0x07 => KeyCode::X,
        0x10 => KeyCode::Y,
        0x06 => KeyCode::Z,

        0x1D => KeyCode::Digit0,
        0x12 => KeyCode::Digit1,
        0x13 => KeyCode::Digit2,
        0x14 => KeyCode::Digit3,
        0x15 => KeyCode::Digit4,
        0x17 => KeyCode::Digit5,
        0x16 => KeyCode::Digit6,
        0x1A => KeyCode::Digit7,
        0x1C => KeyCode::Digit8,
        0x19 => KeyCode::Digit9,

        0x1B => KeyCode::Minus,
        0x18 => KeyCode::Equal,
        0x21 => KeyCode::BracketLeft,
        0x1E => KeyCode::BracketRight,
        0x2A => KeyCode::Backslash,
        0x29 => KeyCode::Semicolon,
        0x27 => KeyCode::Quote,
        0x32 => KeyCode::Backquote,
        0x2B => KeyCode::Comma,
        0x2F => KeyCode::Period,
        0x2C => KeyCode::Slash,
        // `kVK_ISO_Section`: the extra key an ISO keyboard has and an ANSI one
        // does not. It carries `<`, `>` and `\` on a Norwegian layout, so a build
        // that never names it cannot type a backslash.
        0x0A => KeyCode::IntlBackslash,

        0x31 => KeyCode::Space,
        0x24 => KeyCode::Enter,
        0x30 => KeyCode::Tab,
        0x33 => KeyCode::Backspace,
        0x35 => KeyCode::Escape,
        0x75 => KeyCode::Delete,
        0x72 => KeyCode::Insert,
        0x73 => KeyCode::Home,
        0x77 => KeyCode::End,
        0x74 => KeyCode::PageUp,
        0x79 => KeyCode::PageDown,
        0x7E => KeyCode::ArrowUp,
        0x7D => KeyCode::ArrowDown,
        0x7B => KeyCode::ArrowLeft,
        0x7C => KeyCode::ArrowRight,

        0x38 => KeyCode::ShiftLeft,
        0x3C => KeyCode::ShiftRight,
        0x3B => KeyCode::ControlLeft,
        0x3E => KeyCode::ControlRight,
        0x3A => KeyCode::AltLeft,
        0x3D => KeyCode::AltRight,
        0x37 => KeyCode::SuperLeft,
        0x36 => KeyCode::SuperRight,
        0x39 => KeyCode::CapsLock,
        0x47 => KeyCode::NumLock,

        0x7A => KeyCode::F1,
        0x78 => KeyCode::F2,
        0x63 => KeyCode::F3,
        0x76 => KeyCode::F4,
        0x60 => KeyCode::F5,
        0x61 => KeyCode::F6,
        0x62 => KeyCode::F7,
        0x64 => KeyCode::F8,
        0x65 => KeyCode::F9,
        0x6D => KeyCode::F10,
        0x67 => KeyCode::F11,
        0x6F => KeyCode::F12,

        0x52 => KeyCode::Numpad0,
        0x53 => KeyCode::Numpad1,
        0x54 => KeyCode::Numpad2,
        0x55 => KeyCode::Numpad3,
        0x56 => KeyCode::Numpad4,
        0x57 => KeyCode::Numpad5,
        0x58 => KeyCode::Numpad6,
        0x59 => KeyCode::Numpad7,
        0x5B => KeyCode::Numpad8,
        0x5C => KeyCode::Numpad9,
        0x4C => KeyCode::NumpadEnter,
        0x45 => KeyCode::NumpadAdd,
        0x4E => KeyCode::NumpadSubtract,
        0x43 => KeyCode::NumpadMultiply,
        0x4B => KeyCode::NumpadDivide,
        0x41 => KeyCode::NumpadDecimal,

        other => KeyCode::Unidentified(other as u32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two positions mapping to the same key would make one of them untypeable
    /// and the other ambiguous, and neither shows up as an error anywhere.
    #[test]
    fn no_two_virtual_keys_name_the_same_position() {
        let mut seen: Vec<(u16, KeyCode)> = Vec::new();
        for virtual_key in 0u16..=0x7F {
            let code = key_code(virtual_key);
            if matches!(code, KeyCode::Unidentified(_)) {
                continue;
            }
            if let Some((other, _)) = seen.iter().find(|(_, c)| *c == code) {
                panic!("{virtual_key:#04X} and {other:#04X} both name {code:?}");
            }
            seen.push((virtual_key, code));
        }
        assert_eq!(seen.len(), 101, "the table changed size");
    }

    /// Spot checks against `<Carbon/HIToolbox/Events.h>`, chosen for the ones that
    /// are surprising: the letters are not alphabetical, `Z` is not near `Y`, and
    /// the ISO key is a position an ANSI keyboard does not have at all.
    #[test]
    fn the_awkward_codes_are_the_documented_ones() {
        assert_eq!(key_code(0x00), KeyCode::A); // kVK_ANSI_A
        assert_eq!(key_code(0x06), KeyCode::Z); // kVK_ANSI_Z, not near Y
        assert_eq!(key_code(0x10), KeyCode::Y); // kVK_ANSI_Y
        assert_eq!(key_code(0x0A), KeyCode::IntlBackslash); // kVK_ISO_Section
        assert_eq!(key_code(0x1D), KeyCode::Digit0); // 0 comes after 9
        assert_eq!(key_code(0x24), KeyCode::Enter); // kVK_Return
        assert_eq!(key_code(0x33), KeyCode::Backspace); // kVK_Delete, confusingly
        assert_eq!(key_code(0x75), KeyCode::Delete); // kVK_ForwardDelete
    }

    #[test]
    fn an_unknown_code_keeps_its_number() {
        assert_eq!(key_code(0xFF), KeyCode::Unidentified(0xFF));
    }
}
