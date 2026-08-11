//! Key positions, as numbers a C caller can hold.
//!
//! [`KeyCode`] is a Rust enum, so its discriminants are not an ABI and must never
//! become one. This table assigns each position an explicit number that is part of
//! the contract, and the same numbers appear in `denise.h` — checked against each
//! other by [`tests/header.rs`](../tests/header.rs), because two hand-written
//! faces of one table is exactly the sort of thing that drifts.
//!
//! The numbering is not arbitrary. A position is *named* after the US layout, so
//! keys that carry an ASCII character there are numbered with it: `DENISE_KEY_A`
//! is `0x41`, `DENISE_KEY_SEMICOLON` is `0x3B`. That makes a hex dump of a key
//! log readable, and it means a host that already speaks ASCII needs no table for
//! the common half. Everything else lives in a block: `0x100` for named keys,
//! `0x200 + n` for `F<n>`, `0x300 + n` for the numpad digits.

use denise::KeyCode;

/// Set on a key this build cannot name. The low bits carry the raw platform
/// scancode, so a host can still tell two unknown keys apart.
pub const UNIDENTIFIED: u32 = 0x8000_0000;

/// The scancode bits under [`UNIDENTIFIED`].
const RAW_MASK: u32 = 0x7FFF_FFFF;

macro_rules! key_table {
    ($( $c_name:ident = $value:literal => $variant:ident ),* $(,)?) => {
        $(
            #[doc = concat!("[`KeyCode::", stringify!($variant), "`] as an ABI number.")]
            pub const $c_name: u32 = $value;
        )*

        /// The ABI number for a key position.
        ///
        /// A position this table does not list becomes [`UNIDENTIFIED`] with no
        /// scancode. `KeyCode` is `#[non_exhaustive]`, so a position added to the
        /// core after this table was written cannot fail to compile here — it
        /// degrades to "some key, identity unknown" instead, which a host can at
        /// least see.
        pub fn to_abi(code: KeyCode) -> u32 {
            match code {
                $( KeyCode::$variant => $value, )*
                KeyCode::Unidentified(raw) => UNIDENTIFIED | (raw & RAW_MASK),
                _ => UNIDENTIFIED,
            }
        }

        /// The key position for an ABI number, or `None` if it names nothing.
        pub fn from_abi(value: u32) -> Option<KeyCode> {
            match value {
                $( $value => Some(KeyCode::$variant), )*
                v if v & UNIDENTIFIED != 0 => Some(KeyCode::Unidentified(v & RAW_MASK)),
                _ => None,
            }
        }

        /// Every entry, as `(C name, value)`. Used by the header sync test.
        pub const TABLE: &[(&str, u32)] = &[ $( (stringify!($c_name), $value) ),* ];
    };
}

key_table! {
    DENISE_KEY_A = 0x41 => A,
    DENISE_KEY_B = 0x42 => B,
    DENISE_KEY_C = 0x43 => C,
    DENISE_KEY_D = 0x44 => D,
    DENISE_KEY_E = 0x45 => E,
    DENISE_KEY_F = 0x46 => F,
    DENISE_KEY_G = 0x47 => G,
    DENISE_KEY_H = 0x48 => H,
    DENISE_KEY_I = 0x49 => I,
    DENISE_KEY_J = 0x4a => J,
    DENISE_KEY_K = 0x4b => K,
    DENISE_KEY_L = 0x4c => L,
    DENISE_KEY_M = 0x4d => M,
    DENISE_KEY_N = 0x4e => N,
    DENISE_KEY_O = 0x4f => O,
    DENISE_KEY_P = 0x50 => P,
    DENISE_KEY_Q = 0x51 => Q,
    DENISE_KEY_R = 0x52 => R,
    DENISE_KEY_S = 0x53 => S,
    DENISE_KEY_T = 0x54 => T,
    DENISE_KEY_U = 0x55 => U,
    DENISE_KEY_V = 0x56 => V,
    DENISE_KEY_W = 0x57 => W,
    DENISE_KEY_X = 0x58 => X,
    DENISE_KEY_Y = 0x59 => Y,
    DENISE_KEY_Z = 0x5a => Z,
    DENISE_KEY_0 = 0x30 => Digit0,
    DENISE_KEY_1 = 0x31 => Digit1,
    DENISE_KEY_2 = 0x32 => Digit2,
    DENISE_KEY_3 = 0x33 => Digit3,
    DENISE_KEY_4 = 0x34 => Digit4,
    DENISE_KEY_5 = 0x35 => Digit5,
    DENISE_KEY_6 = 0x36 => Digit6,
    DENISE_KEY_7 = 0x37 => Digit7,
    DENISE_KEY_8 = 0x38 => Digit8,
    DENISE_KEY_9 = 0x39 => Digit9,
    DENISE_KEY_SPACE = 0x20 => Space,
    DENISE_KEY_QUOTE = 0x27 => Quote,
    DENISE_KEY_COMMA = 0x2c => Comma,
    DENISE_KEY_MINUS = 0x2d => Minus,
    DENISE_KEY_PERIOD = 0x2e => Period,
    DENISE_KEY_SLASH = 0x2f => Slash,
    DENISE_KEY_SEMICOLON = 0x3b => Semicolon,
    DENISE_KEY_EQUAL = 0x3d => Equal,
    DENISE_KEY_BRACKET_LEFT = 0x5b => BracketLeft,
    DENISE_KEY_BACKSLASH = 0x5c => Backslash,
    DENISE_KEY_BRACKET_RIGHT = 0x5d => BracketRight,
    DENISE_KEY_BACKQUOTE = 0x60 => Backquote,
    DENISE_KEY_ESCAPE = 0x100 => Escape,
    DENISE_KEY_ENTER = 0x101 => Enter,
    DENISE_KEY_TAB = 0x102 => Tab,
    DENISE_KEY_BACKSPACE = 0x103 => Backspace,
    DENISE_KEY_DELETE = 0x104 => Delete,
    DENISE_KEY_INSERT = 0x105 => Insert,
    DENISE_KEY_HOME = 0x106 => Home,
    DENISE_KEY_END = 0x107 => End,
    DENISE_KEY_PAGE_UP = 0x108 => PageUp,
    DENISE_KEY_PAGE_DOWN = 0x109 => PageDown,
    DENISE_KEY_ARROW_UP = 0x10a => ArrowUp,
    DENISE_KEY_ARROW_DOWN = 0x10b => ArrowDown,
    DENISE_KEY_ARROW_LEFT = 0x10c => ArrowLeft,
    DENISE_KEY_ARROW_RIGHT = 0x10d => ArrowRight,
    DENISE_KEY_CAPS_LOCK = 0x10e => CapsLock,
    DENISE_KEY_NUM_LOCK = 0x10f => NumLock,
    DENISE_KEY_SCROLL_LOCK = 0x110 => ScrollLock,
    DENISE_KEY_INTL_BACKSLASH = 0x111 => IntlBackslash,
    DENISE_KEY_SHIFT_LEFT = 0x120 => ShiftLeft,
    DENISE_KEY_SHIFT_RIGHT = 0x121 => ShiftRight,
    DENISE_KEY_CONTROL_LEFT = 0x122 => ControlLeft,
    DENISE_KEY_CONTROL_RIGHT = 0x123 => ControlRight,
    DENISE_KEY_ALT_LEFT = 0x124 => AltLeft,
    DENISE_KEY_ALT_RIGHT = 0x125 => AltRight,
    DENISE_KEY_SUPER_LEFT = 0x126 => SuperLeft,
    DENISE_KEY_SUPER_RIGHT = 0x127 => SuperRight,
    DENISE_KEY_F1 = 0x201 => F1,
    DENISE_KEY_F2 = 0x202 => F2,
    DENISE_KEY_F3 = 0x203 => F3,
    DENISE_KEY_F4 = 0x204 => F4,
    DENISE_KEY_F5 = 0x205 => F5,
    DENISE_KEY_F6 = 0x206 => F6,
    DENISE_KEY_F7 = 0x207 => F7,
    DENISE_KEY_F8 = 0x208 => F8,
    DENISE_KEY_F9 = 0x209 => F9,
    DENISE_KEY_F10 = 0x20a => F10,
    DENISE_KEY_F11 = 0x20b => F11,
    DENISE_KEY_F12 = 0x20c => F12,
    DENISE_KEY_NUMPAD_0 = 0x300 => Numpad0,
    DENISE_KEY_NUMPAD_1 = 0x301 => Numpad1,
    DENISE_KEY_NUMPAD_2 = 0x302 => Numpad2,
    DENISE_KEY_NUMPAD_3 = 0x303 => Numpad3,
    DENISE_KEY_NUMPAD_4 = 0x304 => Numpad4,
    DENISE_KEY_NUMPAD_5 = 0x305 => Numpad5,
    DENISE_KEY_NUMPAD_6 = 0x306 => Numpad6,
    DENISE_KEY_NUMPAD_7 = 0x307 => Numpad7,
    DENISE_KEY_NUMPAD_8 = 0x308 => Numpad8,
    DENISE_KEY_NUMPAD_9 = 0x309 => Numpad9,
    DENISE_KEY_NUMPAD_ENTER = 0x30a => NumpadEnter,
    DENISE_KEY_NUMPAD_ADD = 0x30b => NumpadAdd,
    DENISE_KEY_NUMPAD_SUBTRACT = 0x30c => NumpadSubtract,
    DENISE_KEY_NUMPAD_MULTIPLY = 0x30d => NumpadMultiply,
    DENISE_KEY_NUMPAD_DIVIDE = 0x30e => NumpadDivide,
    DENISE_KEY_NUMPAD_DECIMAL = 0x30f => NumpadDecimal,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_round_trips() {
        for &(name, value) in TABLE {
            let code = from_abi(value).unwrap_or_else(|| panic!("{name} = {value:#x} unmapped"));
            assert_eq!(to_abi(code), value, "{name} did not survive the round trip");
        }
    }

    /// A duplicate in `from_abi` is an unreachable match arm and rustc says so; a
    /// duplicate in `to_abi` is silent, and would make two positions
    /// indistinguishable to a host.
    #[test]
    fn values_and_names_are_distinct() {
        let mut values: Vec<u32> = TABLE.iter().map(|&(_, v)| v).collect();
        values.sort_unstable();
        let before = values.len();
        values.dedup();
        assert_eq!(values.len(), before, "two keys share a number");

        let mut names: Vec<&str> = TABLE.iter().map(|&(n, _)| n).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "two keys share a name");
    }

    /// The escape hatch has to stay outside the numbered range, or a real key
    /// would arrive at the host wearing the "unknown key" flag.
    #[test]
    fn no_named_key_collides_with_the_unidentified_flag() {
        for &(name, value) in TABLE {
            assert_eq!(value & UNIDENTIFIED, 0, "{name} sets the unidentified bit");
        }
    }

    #[test]
    fn an_unnamed_key_carries_its_scancode() {
        // 194 is KEY_YEN on Linux, which this build does not name.
        let code = KeyCode::Unidentified(194);
        assert_eq!(to_abi(code), UNIDENTIFIED | 194);
        assert_eq!(from_abi(UNIDENTIFIED | 194), Some(code));
    }

    #[test]
    fn a_number_that_names_nothing_is_rejected() {
        assert_eq!(from_abi(0), None);
        assert_eq!(from_abi(0x400), None);
    }

    /// Spot checks on the promise the numbering makes, because the point of it is
    /// that a host can rely on it rather than look it up.
    #[test]
    fn ascii_positions_are_numbered_with_their_ascii() {
        assert_eq!(to_abi(KeyCode::A), u32::from(b'A'));
        assert_eq!(to_abi(KeyCode::Digit7), u32::from(b'7'));
        assert_eq!(to_abi(KeyCode::Semicolon), u32::from(b';'));
        assert_eq!(to_abi(KeyCode::Space), u32::from(b' '));
        assert_eq!(to_abi(KeyCode::F5), 0x200 + 5);
        assert_eq!(to_abi(KeyCode::Numpad3), 0x300 + 3);
    }
}
