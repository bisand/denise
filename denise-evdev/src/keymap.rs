//! evdev key codes to [`KeyCode`] positions.
//!
//! evdev codes name physical key *positions* using US-layout labels, which is
//! exactly what [`KeyCode`] means, so this is a straight table with no layout
//! interpretation. What the key produces is a separate question answered by
//! [`denise::InputEvent::Text`]: `KeyCode::Semicolon` is where `ø` lives on a
//! Norwegian keyboard, and mapping it to a character here would be wrong on every
//! layout but one.

use denise::KeyCode;

/// Translates a raw evdev key code.
///
/// Unknown codes become [`KeyCode::Unidentified`] carrying the raw value, so a key
/// this table does not name is still distinguishable and still reportable.
pub fn key_code(code: u16) -> KeyCode {
    use KeyCode as K;

    match code {
        // Letters, in evdev's keyboard-row order rather than alphabetical.
        30 => K::A,
        48 => K::B,
        46 => K::C,
        32 => K::D,
        18 => K::E,
        33 => K::F,
        34 => K::G,
        35 => K::H,
        23 => K::I,
        36 => K::J,
        37 => K::K,
        38 => K::L,
        50 => K::M,
        49 => K::N,
        24 => K::O,
        25 => K::P,
        16 => K::Q,
        19 => K::R,
        31 => K::S,
        20 => K::T,
        22 => K::U,
        47 => K::V,
        17 => K::W,
        45 => K::X,
        21 => K::Y,
        44 => K::Z,

        // Digit row. Note evdev puts 0 at the end, after 9.
        11 => K::Digit0,
        2 => K::Digit1,
        3 => K::Digit2,
        4 => K::Digit3,
        5 => K::Digit4,
        6 => K::Digit5,
        7 => K::Digit6,
        8 => K::Digit7,
        9 => K::Digit8,
        10 => K::Digit9,

        59 => K::F1,
        60 => K::F2,
        61 => K::F3,
        62 => K::F4,
        63 => K::F5,
        64 => K::F6,
        65 => K::F7,
        66 => K::F8,
        67 => K::F9,
        68 => K::F10,
        87 => K::F11,
        88 => K::F12,

        1 => K::Escape,
        28 => K::Enter,
        15 => K::Tab,
        57 => K::Space,
        14 => K::Backspace,
        110 => K::Insert,
        111 => K::Delete,
        102 => K::Home,
        107 => K::End,
        104 => K::PageUp,
        109 => K::PageDown,
        103 => K::ArrowUp,
        108 => K::ArrowDown,
        105 => K::ArrowLeft,
        106 => K::ArrowRight,

        42 => K::ShiftLeft,
        54 => K::ShiftRight,
        29 => K::ControlLeft,
        97 => K::ControlRight,
        56 => K::AltLeft,
        // AltGr. Reported as a distinct position, which is what makes the third
        // level of a Norwegian layout reachable at all.
        100 => K::AltRight,
        125 => K::SuperLeft,
        126 => K::SuperRight,
        58 => K::CapsLock,
        69 => K::NumLock,
        70 => K::ScrollLock,

        12 => K::Minus,
        13 => K::Equal,
        26 => K::BracketLeft,
        27 => K::BracketRight,
        43 => K::Backslash,
        39 => K::Semicolon,
        40 => K::Quote,
        41 => K::Backquote,
        // The 102nd key, present on ISO keyboards and absent on ANSI ones.
        86 => K::IntlBackslash,
        51 => K::Comma,
        52 => K::Period,
        53 => K::Slash,

        96 => K::NumpadEnter,
        78 => K::NumpadAdd,
        74 => K::NumpadSubtract,
        55 => K::NumpadMultiply,
        98 => K::NumpadDivide,
        83 => K::NumpadDecimal,
        82 => K::Numpad0,
        79 => K::Numpad1,
        80 => K::Numpad2,
        81 => K::Numpad3,
        75 => K::Numpad4,
        76 => K::Numpad5,
        77 => K::Numpad6,
        71 => K::Numpad7,
        72 => K::Numpad8,
        73 => K::Numpad9,

        other => K::Unidentified(u32::from(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_row_maps_to_the_right_letters() {
        assert_eq!(key_code(30), KeyCode::A);
        assert_eq!(key_code(31), KeyCode::S);
        assert_eq!(key_code(32), KeyCode::D);
        assert_eq!(key_code(33), KeyCode::F);
    }

    #[test]
    fn digit_zero_sits_after_nine_not_before_one() {
        // The off-by-one that a loop over the digit row would introduce.
        assert_eq!(key_code(2), KeyCode::Digit1);
        assert_eq!(key_code(10), KeyCode::Digit9);
        assert_eq!(key_code(11), KeyCode::Digit0);
    }

    #[test]
    fn altgr_is_distinct_from_left_alt() {
        // Collapsing these breaks the third level of a Norwegian layout, where
        // AltGr is how you reach @, $ and the braces.
        assert_eq!(key_code(56), KeyCode::AltLeft);
        assert_eq!(key_code(100), KeyCode::AltRight);
        assert_ne!(key_code(56), key_code(100));
    }

    #[test]
    fn the_keys_norwegian_letters_live_on_are_positions_not_characters() {
        // On a Norwegian layout these three positions carry ø, æ and å. The map
        // must report the position; the character comes from text input.
        assert_eq!(key_code(39), KeyCode::Semicolon);
        assert_eq!(key_code(40), KeyCode::Quote);
        assert_eq!(key_code(26), KeyCode::BracketLeft);
    }

    #[test]
    fn unknown_codes_keep_their_raw_value() {
        assert_eq!(key_code(0xFFF), KeyCode::Unidentified(0xFFF));
    }

    #[test]
    fn no_two_named_codes_collide() {
        // A duplicated arm would silently shadow a key, and the compiler only
        // warns about unreachable patterns for literals it can prove overlap.
        let mut seen = std::collections::HashMap::new();
        for code in 0u16..=255 {
            let mapped = key_code(code);
            if matches!(mapped, KeyCode::Unidentified(_)) {
                continue;
            }
            if let Some(previous) = seen.insert(mapped, code) {
                panic!("codes {previous} and {code} both map to {mapped:?}");
            }
        }
    }
}
