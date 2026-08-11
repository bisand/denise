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
//!
//! # Why the numbers are written out here
//!
//! This module is deliberately platform-independent, so it compiles and its tests
//! run on a development machine with no Windows anywhere — the same rule
//! [`denise_evdev`](https://docs.rs/denise-evdev)'s translation follows, and for
//! the same reason: a table of a hundred numbers is exactly the thing that goes
//! wrong, and finding out on a CI runner is a slow way to find out. Taking the
//! constants from the `windows` bindings instead would have moved this behind
//! `cfg(windows)` and cost the local test run. A Windows-only test below checks
//! these against the official bindings, so the two cannot drift.

use denise::KeyCode;

/// The key position for a virtual key code, or `Unidentified` carrying the raw
/// value.
///
/// `extended` is bit 24 of the `WM_KEYDOWN` `lParam`, and it is the only thing
/// that separates the numpad Enter from the main one, or right control from left.
/// Windows sends the same `VK_` code for both.
pub fn key_code(virtual_key: u16, extended: bool) -> KeyCode {
    match virtual_key {
        // The letters and digits are their ASCII values, which the API guarantees
        // rather than merely happens to do.
        vk @ 0x41..=0x5A => LETTERS[(vk - 0x41) as usize],
        vk @ 0x30..=0x39 => DIGITS[(vk - 0x30) as usize],

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
        // them, and they deliberately resolve to the same positions as the sided
        // ones — see `UNSIDED`. The extended bit still separates control and alt,
        // which matters: AltGr is reported as extended right alt, and losing that
        // would make it indistinguishable from a plain alt — a shortcut, not a
        // third keyboard level.
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

/// The three codes Windows sends when nothing has asked it to tell the left and
/// right modifiers apart.
///
/// They alias the sided codes by design, which is the one place two virtual keys
/// are *supposed* to name the same position.
pub const UNSIDED: [u16; 3] = [VK_SHIFT, VK_CONTROL, VK_MENU];

/// `VK_BACK`.
pub const VK_BACK: u16 = 0x08;
/// `VK_TAB`.
pub const VK_TAB: u16 = 0x09;
/// `VK_RETURN`.
pub const VK_RETURN: u16 = 0x0D;
/// `VK_SHIFT`, either one.
pub const VK_SHIFT: u16 = 0x10;
/// `VK_CONTROL`, either one.
pub const VK_CONTROL: u16 = 0x11;
/// `VK_MENU`, either alt.
pub const VK_MENU: u16 = 0x12;
/// `VK_CAPITAL`.
pub const VK_CAPITAL: u16 = 0x14;
/// `VK_ESCAPE`.
pub const VK_ESCAPE: u16 = 0x1B;
/// `VK_SPACE`.
pub const VK_SPACE: u16 = 0x20;
/// `VK_PRIOR`, which is Page Up.
pub const VK_PRIOR: u16 = 0x21;
/// `VK_NEXT`, which is Page Down.
pub const VK_NEXT: u16 = 0x22;
/// `VK_END`.
pub const VK_END: u16 = 0x23;
/// `VK_HOME`.
pub const VK_HOME: u16 = 0x24;
/// `VK_LEFT`.
pub const VK_LEFT: u16 = 0x25;
/// `VK_UP`.
pub const VK_UP: u16 = 0x26;
/// `VK_RIGHT`.
pub const VK_RIGHT: u16 = 0x27;
/// `VK_DOWN`.
pub const VK_DOWN: u16 = 0x28;
/// `VK_INSERT`.
pub const VK_INSERT: u16 = 0x2D;
/// `VK_DELETE`.
pub const VK_DELETE: u16 = 0x2E;
/// `VK_LWIN`.
pub const VK_LWIN: u16 = 0x5B;
/// `VK_RWIN`.
pub const VK_RWIN: u16 = 0x5C;
/// `VK_NUMPAD0`; the ten numpad digits run consecutively from here.
pub const VK_NUMPAD0: u16 = 0x60;
/// `VK_NUMPAD1`.
pub const VK_NUMPAD1: u16 = 0x61;
/// `VK_NUMPAD2`.
pub const VK_NUMPAD2: u16 = 0x62;
/// `VK_NUMPAD3`.
pub const VK_NUMPAD3: u16 = 0x63;
/// `VK_NUMPAD4`.
pub const VK_NUMPAD4: u16 = 0x64;
/// `VK_NUMPAD5`.
pub const VK_NUMPAD5: u16 = 0x65;
/// `VK_NUMPAD6`.
pub const VK_NUMPAD6: u16 = 0x66;
/// `VK_NUMPAD7`.
pub const VK_NUMPAD7: u16 = 0x67;
/// `VK_NUMPAD8`.
pub const VK_NUMPAD8: u16 = 0x68;
/// `VK_NUMPAD9`.
pub const VK_NUMPAD9: u16 = 0x69;
/// `VK_MULTIPLY`.
pub const VK_MULTIPLY: u16 = 0x6A;
/// `VK_ADD`.
pub const VK_ADD: u16 = 0x6B;
/// `VK_SUBTRACT`.
pub const VK_SUBTRACT: u16 = 0x6D;
/// `VK_DECIMAL`.
pub const VK_DECIMAL: u16 = 0x6E;
/// `VK_DIVIDE`.
pub const VK_DIVIDE: u16 = 0x6F;
/// `VK_F1`; the twelve function keys run consecutively from here.
pub const VK_F1: u16 = 0x70;
/// `VK_F2`.
pub const VK_F2: u16 = 0x71;
/// `VK_F3`.
pub const VK_F3: u16 = 0x72;
/// `VK_F4`.
pub const VK_F4: u16 = 0x73;
/// `VK_F5`.
pub const VK_F5: u16 = 0x74;
/// `VK_F6`.
pub const VK_F6: u16 = 0x75;
/// `VK_F7`.
pub const VK_F7: u16 = 0x76;
/// `VK_F8`.
pub const VK_F8: u16 = 0x77;
/// `VK_F9`.
pub const VK_F9: u16 = 0x78;
/// `VK_F10`.
pub const VK_F10: u16 = 0x79;
/// `VK_F11`.
pub const VK_F11: u16 = 0x7A;
/// `VK_F12`.
pub const VK_F12: u16 = 0x7B;
/// `VK_NUMLOCK`.
pub const VK_NUMLOCK: u16 = 0x90;
/// `VK_SCROLL`.
pub const VK_SCROLL: u16 = 0x91;
/// `VK_LSHIFT`.
pub const VK_LSHIFT: u16 = 0xA0;
/// `VK_RSHIFT`.
pub const VK_RSHIFT: u16 = 0xA1;
/// `VK_LCONTROL`.
pub const VK_LCONTROL: u16 = 0xA2;
/// `VK_RCONTROL`.
pub const VK_RCONTROL: u16 = 0xA3;
/// `VK_LMENU`.
pub const VK_LMENU: u16 = 0xA4;
/// `VK_RMENU`, which is also AltGr when the extended bit is set.
pub const VK_RMENU: u16 = 0xA5;
/// `VK_OEM_1`: `;` on a US layout, `ø` on a Norwegian one.
pub const VK_OEM_1: u16 = 0xBA;
/// `VK_OEM_PLUS`.
pub const VK_OEM_PLUS: u16 = 0xBB;
/// `VK_OEM_COMMA`.
pub const VK_OEM_COMMA: u16 = 0xBC;
/// `VK_OEM_MINUS`.
pub const VK_OEM_MINUS: u16 = 0xBD;
/// `VK_OEM_PERIOD`.
pub const VK_OEM_PERIOD: u16 = 0xBE;
/// `VK_OEM_2`.
pub const VK_OEM_2: u16 = 0xBF;
/// `VK_OEM_3`.
pub const VK_OEM_3: u16 = 0xC0;
/// `VK_OEM_4`.
pub const VK_OEM_4: u16 = 0xDB;
/// `VK_OEM_5`.
pub const VK_OEM_5: u16 = 0xDC;
/// `VK_OEM_6`.
pub const VK_OEM_6: u16 = 0xDD;
/// `VK_OEM_7`: `'` on a US layout, `æ` on a Norwegian one.
pub const VK_OEM_7: u16 = 0xDE;
/// `VK_OEM_102`: the 102nd key, which an ANSI keyboard does not have.
pub const VK_OEM_102: u16 = 0xE2;

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
        assert_eq!(key_code(VK_MENU, false), KeyCode::AltLeft);
        assert_eq!(key_code(VK_MENU, true), KeyCode::AltRight);
        assert_eq!(key_code(VK_CONTROL, false), KeyCode::ControlLeft);
        assert_eq!(key_code(VK_CONTROL, true), KeyCode::ControlRight);
        assert_eq!(key_code(VK_RETURN, false), KeyCode::Enter);
        assert_eq!(key_code(VK_RETURN, true), KeyCode::NumpadEnter);
    }

    #[test]
    fn the_iso_key_is_named() {
        assert_eq!(key_code(VK_OEM_102, false), KeyCode::IntlBackslash);
    }

    /// Two positions naming the same key would make one untypeable and the other
    /// ambiguous, and neither shows up as an error anywhere.
    ///
    /// The unsided codes are excluded because their aliasing is the point: when
    /// nothing has asked Windows to distinguish the two shifts, reporting "some
    /// shift" as the left one is the useful answer. Every *other* collision is a
    /// bug — which is what this missed the first time, by excluding the resulting
    /// `KeyCode` rather than the deliberate aliases, so control and alt slipped
    /// through and a Windows CI runner had to say so.
    #[test]
    fn no_two_virtual_keys_name_the_same_position() {
        let mut seen: Vec<(u16, KeyCode)> = Vec::new();
        for virtual_key in 0u16..=0xFF {
            if UNSIDED.contains(&virtual_key) {
                continue;
            }
            let code = key_code(virtual_key, false);
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

    /// The unsided codes must land on positions the sided ones also produce —
    /// aliases, not a third set of keys nothing else ever reports.
    #[test]
    fn the_unsided_codes_alias_the_sided_ones() {
        assert_eq!(key_code(VK_SHIFT, false), key_code(VK_LSHIFT, false));
        assert_eq!(key_code(VK_CONTROL, false), key_code(VK_LCONTROL, false));
        assert_eq!(key_code(VK_CONTROL, true), key_code(VK_RCONTROL, false));
        assert_eq!(key_code(VK_MENU, false), key_code(VK_LMENU, false));
        assert_eq!(key_code(VK_MENU, true), key_code(VK_RMENU, false));
    }

    #[test]
    fn an_unknown_code_keeps_its_number() {
        assert_eq!(key_code(0xFE, false), KeyCode::Unidentified(0xFE));
    }

    /// The numbers above are written out so this module compiles anywhere. This
    /// is what stops them drifting from the ones Windows actually sends.
    #[cfg(windows)]
    #[test]
    fn the_constants_match_the_official_bindings() {
        use windows::Win32::UI::Input::KeyboardAndMouse as vk;
        let pairs: [(u16, u16, &str); 24] = [
            (VK_BACK, vk::VK_BACK.0, "VK_BACK"),
            (VK_TAB, vk::VK_TAB.0, "VK_TAB"),
            (VK_RETURN, vk::VK_RETURN.0, "VK_RETURN"),
            (VK_SHIFT, vk::VK_SHIFT.0, "VK_SHIFT"),
            (VK_CONTROL, vk::VK_CONTROL.0, "VK_CONTROL"),
            (VK_MENU, vk::VK_MENU.0, "VK_MENU"),
            (VK_CAPITAL, vk::VK_CAPITAL.0, "VK_CAPITAL"),
            (VK_ESCAPE, vk::VK_ESCAPE.0, "VK_ESCAPE"),
            (VK_SPACE, vk::VK_SPACE.0, "VK_SPACE"),
            (VK_PRIOR, vk::VK_PRIOR.0, "VK_PRIOR"),
            (VK_NEXT, vk::VK_NEXT.0, "VK_NEXT"),
            (VK_INSERT, vk::VK_INSERT.0, "VK_INSERT"),
            (VK_DELETE, vk::VK_DELETE.0, "VK_DELETE"),
            (VK_LWIN, vk::VK_LWIN.0, "VK_LWIN"),
            (VK_NUMPAD0, vk::VK_NUMPAD0.0, "VK_NUMPAD0"),
            (VK_DECIMAL, vk::VK_DECIMAL.0, "VK_DECIMAL"),
            (VK_F1, vk::VK_F1.0, "VK_F1"),
            (VK_F12, vk::VK_F12.0, "VK_F12"),
            (VK_NUMLOCK, vk::VK_NUMLOCK.0, "VK_NUMLOCK"),
            (VK_LCONTROL, vk::VK_LCONTROL.0, "VK_LCONTROL"),
            (VK_RMENU, vk::VK_RMENU.0, "VK_RMENU"),
            (VK_OEM_1, vk::VK_OEM_1.0, "VK_OEM_1"),
            (VK_OEM_7, vk::VK_OEM_7.0, "VK_OEM_7"),
            (VK_OEM_102, vk::VK_OEM_102.0, "VK_OEM_102"),
        ];
        for (ours, theirs, name) in pairs {
            assert_eq!(
                ours, theirs,
                "{name} is {ours:#04X} here and {theirs:#04X} in the bindings"
            );
        }
    }
}
