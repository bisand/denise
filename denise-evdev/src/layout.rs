//! Key positions to characters: layouts, dead keys and composition.
//!
//! [`crate::keymap`] answers *where* a key is; this answers *what it types*. The
//! split matters because a position is a fact about the hardware and a character
//! is a fact about the user's layout, and conflating them is how a toolkit ends up
//! unable to type `ø` on the machine it was written for.
//!
//! Everything here is platform-independent and allocation-free, so it is unit
//! tested rather than inferred from a keyboard someone happened to have plugged
//! in.
//!
//! # Using the system's layout
//!
//! [`from_system`] reads what the machine is already configured for —
//! `DENISE_KEYMAP`, then `XKB_DEFAULT_LAYOUT`, then the console keyboard
//! configuration files distributions actually write. On the Raspberry Pi this was
//! developed against, `/etc/conf.d/loadkmap` says `no` and the panel picks it up
//! with nothing set by hand.
//!
//! That reads the system's *choice*. Reading the system's *layout data* is a
//! different question, and the reason this crate carries its own tables:
//!
//! - **The kernel's own keymap**, via `KDGKBENT` and `KDGKBDIACRUC` on a VT, is
//!   the technically right answer and is not much code. It needs `/dev/tty0`,
//!   which is `root:root` mode 600 on every distribution checked. Denise
//!   otherwise runs unprivileged, needing only the `video` and `input` groups,
//!   and giving that up to read a keymap is a poor trade.
//! - **libxkbcommon** is the correct answer on a desktop and the wrong one here:
//!   a C library with a runtime data directory, which defeats "one static binary"
//!   on a read-only root.
//!
//! So the choice comes from the system and the data comes from here. The cost is
//! that a system configured for a layout Denise has no table for falls back to
//! US — visibly, through [`LayoutSource`], rather than by typing the wrong thing.
//! Adding a table is about thirty lines; needing root is forever.
//!
//! # Control characters are never text
//!
//! Enter, Tab and Backspace produce [`InputEvent::Key`] and nothing else.
//! [`InputEvent::Text`] carries characters a user meant to insert, so a text field
//! can insert everything it receives without filtering, and a key binding cannot
//! be shadowed by a stray control character.
//!
//! [`InputEvent::Key`]: denise::InputEvent::Key
//! [`InputEvent::Text`]: denise::InputEvent::Text

use denise::{ElementState, KeyCode, Modifiers};

/// What one position produces at one shift level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Output {
    /// Nothing. The position is unused at this level.
    #[default]
    None,
    /// A character, inserted directly.
    Char(char),
    /// A dead key, held until the next character decides what it becomes.
    ///
    /// Carries the *spacing* form of the mark — `'¨'`, not the combining
    /// U+0308 — because that is what gets emitted when the composition fails or
    /// the user types the mark twice.
    Dead(char),
}

impl Output {
    #[inline]
    const fn is_none(self) -> bool {
        matches!(self, Output::None)
    }
}

/// One physical position and what it types at each of four levels.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    /// The position this describes.
    pub code: KeyCode,
    /// Unmodified.
    pub base: Output,
    /// With Shift.
    pub shift: Output,
    /// With AltGr, the third level.
    pub altgr: Output,
    /// With Shift and AltGr.
    pub shift_altgr: Output,
}

impl Entry {
    /// A position with only two levels.
    const fn pair(code: KeyCode, base: char, shift: char) -> Self {
        Self {
            code,
            base: Output::Char(base),
            shift: Output::Char(shift),
            altgr: Output::None,
            shift_altgr: Output::None,
        }
    }

    /// A position with a third level on AltGr.
    const fn triple(code: KeyCode, base: char, shift: char, altgr: char) -> Self {
        Self {
            code,
            base: Output::Char(base),
            shift: Output::Char(shift),
            altgr: Output::Char(altgr),
            shift_altgr: Output::None,
        }
    }

    /// A letter, whose two levels are its two cases.
    const fn letter(code: KeyCode, lower: char, upper: char) -> Self {
        Self::pair(code, lower, upper)
    }

    #[inline]
    const fn at(&self, shift: bool, level3: bool) -> Output {
        match (shift, level3) {
            (false, false) => self.base,
            (true, false) => self.shift,
            (false, true) => self.altgr,
            (true, true) => {
                // Most positions have nothing on the fourth level, and falling
                // back to the third is what every real layout does there.
                if self.shift_altgr.is_none() {
                    self.altgr
                } else {
                    self.shift_altgr
                }
            }
        }
    }
}

/// A keyboard layout: a table of positions, and the decimal key's character.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    /// Human-readable name, for logging what a device is being read as.
    pub name: &'static str,
    /// Positions, in no particular order. Looked up by linear scan: about fifty
    /// comparisons, at most a few times per second, against the complexity of
    /// keeping a sorted table sorted.
    pub entries: &'static [Entry],
    /// What the numpad's decimal key types. `.` in most of the world, `,` in most
    /// of Europe.
    pub decimal_separator: char,
}

impl Layout {
    fn entry(&self, code: KeyCode) -> Option<&'static Entry> {
        // The layout's own table wins, so a layout that needs a different letter
        // overrides it simply by listing that position.
        self.entries
            .iter()
            .find(|entry| entry.code == code)
            .or_else(|| LETTERS.iter().find(|entry| entry.code == code))
    }
}

/// Characters produced by one keystroke: never more than two.
///
/// Two happens when a dead key is followed by something it cannot combine with —
/// `¨` then `q` gives `¨q`, which is what every desktop does and is far better
/// than silently dropping the mark the user typed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Composed {
    chars: [char; 2],
    len: u8,
}

impl Composed {
    /// Nothing to insert.
    pub const NONE: Self = Self {
        chars: ['\0', '\0'],
        len: 0,
    };

    const fn one(ch: char) -> Self {
        Self {
            chars: [ch, '\0'],
            len: 1,
        }
    }

    const fn two(first: char, second: char) -> Self {
        Self {
            chars: [first, second],
            len: 2,
        }
    }

    /// The characters, in the order they should be inserted.
    #[inline]
    pub fn as_slice(&self) -> &[char] {
        &self.chars[..self.len as usize]
    }

    /// Returns `true` if nothing was produced.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Turns key transitions into characters, holding the state a layout needs.
///
/// Owns the dead-key latch, Caps Lock, Num Lock and the AltGr level, because all
/// four are *sequences* rather than properties of a single event and none of them
/// can be recovered from one keystroke in isolation.
#[derive(Clone, Debug)]
pub struct Composer {
    layout: &'static Layout,
    pending_dead: Option<char>,
    caps_lock: bool,
    num_lock: bool,
    /// AltGr is held. Tracked here rather than read from [`Modifiers`] because
    /// `Modifiers::ALT` cannot tell the two Alt keys apart, and on an ISO layout
    /// only the right one reaches the third level.
    level3: bool,
}

impl Composer {
    /// A composer for `layout`, with Num Lock on as a keyboard reports it after a
    /// cold boot on most firmware.
    pub fn new(layout: &'static Layout) -> Self {
        Self {
            layout,
            pending_dead: None,
            caps_lock: false,
            num_lock: true,
            level3: false,
        }
    }

    /// The active layout.
    #[inline]
    pub const fn layout(&self) -> &'static Layout {
        self.layout
    }

    /// Switches layout, abandoning any half-finished composition.
    pub fn set_layout(&mut self, layout: &'static Layout) {
        self.layout = layout;
        self.pending_dead = None;
    }

    /// The mark waiting for a base character, if any.
    #[inline]
    pub const fn pending_dead(&self) -> Option<char> {
        self.pending_dead
    }

    /// Whether Caps Lock is latched.
    #[inline]
    pub const fn caps_lock(&self) -> bool {
        self.caps_lock
    }

    /// Feeds one key transition and returns what it types.
    ///
    /// `modifiers` is the state *including* this key, as the translator reports it.
    pub fn feed(&mut self, code: KeyCode, state: ElementState, modifiers: Modifiers) -> Composed {
        if code == KeyCode::AltRight {
            self.level3 = state.is_down();
            return Composed::NONE;
        }
        if state != ElementState::Down {
            return Composed::NONE;
        }
        match code {
            KeyCode::CapsLock => {
                self.caps_lock = !self.caps_lock;
                return Composed::NONE;
            }
            KeyCode::NumLock => {
                self.num_lock = !self.num_lock;
                return Composed::NONE;
            }
            _ => {}
        }

        // Ctrl or a plain Alt means a binding, not text. AltGr is neither, and
        // while it is held it overrides both — because a great many keyboards and
        // firmwares report AltGr as Ctrl plus Alt, and a rule that let Ctrl veto
        // text would silently disable the whole third level on exactly those.
        // Super still suppresses: nothing sends it alongside AltGr.
        let chord = if self.level3 {
            modifiers.contains(Modifiers::SUPER)
        } else {
            modifiers.contains(Modifiers::CTRL)
                || modifiers.contains(Modifiers::SUPER)
                || modifiers.contains(Modifiers::ALT)
        };
        if chord {
            self.pending_dead = None;
            return Composed::NONE;
        }

        let shift = modifiers.contains(Modifiers::SHIFT);
        let output = self.output_for(code, shift);
        match output {
            Output::None => {
                // Anything that types nothing cancels a half-finished composition,
                // so Escape or an arrow key leaves no latch behind to surprise the
                // next keystroke.
                self.pending_dead = None;
                Composed::NONE
            }
            Output::Dead(mark) => match self.pending_dead.replace(mark) {
                // The same mark twice is how every layout types the mark itself.
                Some(previous) if previous == mark => {
                    self.pending_dead = None;
                    Composed::one(mark)
                }
                Some(previous) => Composed::one(previous),
                None => Composed::NONE,
            },
            Output::Char(ch) => match self.pending_dead.take() {
                None => Composed::one(ch),
                // Space is the conventional way to ask for the bare mark.
                Some(mark) if ch == ' ' => Composed::one(mark),
                Some(mark) => match compose(mark, ch) {
                    Some(combined) => Composed::one(combined),
                    None => Composed::two(mark, ch),
                },
            },
        }
    }

    fn output_for(&self, code: KeyCode, shift: bool) -> Output {
        if let Some(output) = self.numpad(code) {
            return output;
        }
        if code == KeyCode::Space {
            return Output::Char(' ');
        }
        let Some(entry) = self.layout.entry(code) else {
            return Output::None;
        };
        // Caps Lock inverts shift for letters only. Applying it to the digit row
        // is the bug that makes a locked keyboard type `!` for `1`.
        let shift = shift != (self.caps_lock && is_letter(entry));
        entry.at(shift, self.level3)
    }

    fn numpad(&self, code: KeyCode) -> Option<Output> {
        let digit = match code {
            KeyCode::Numpad0 => '0',
            KeyCode::Numpad1 => '1',
            KeyCode::Numpad2 => '2',
            KeyCode::Numpad3 => '3',
            KeyCode::Numpad4 => '4',
            KeyCode::Numpad5 => '5',
            KeyCode::Numpad6 => '6',
            KeyCode::Numpad7 => '7',
            KeyCode::Numpad8 => '8',
            KeyCode::Numpad9 => '9',
            KeyCode::NumpadDecimal => self.layout.decimal_separator,
            KeyCode::NumpadAdd => return Some(Output::Char('+')),
            KeyCode::NumpadSubtract => return Some(Output::Char('-')),
            KeyCode::NumpadMultiply => return Some(Output::Char('*')),
            KeyCode::NumpadDivide => return Some(Output::Char('/')),
            _ => return None,
        };
        // With Num Lock off the numpad is arrows and Home/End, which are positions
        // and not text at all.
        Some(if self.num_lock {
            Output::Char(digit)
        } else {
            Output::None
        })
    }
}

/// Returns `true` if both of an entry's first two levels are cased letters.
fn is_letter(entry: &Entry) -> bool {
    matches!(
        (entry.base, entry.shift),
        (Output::Char(lower), Output::Char(upper))
            if lower.is_alphabetic() && upper.is_alphabetic()
    )
}

/// Combines a dead mark with a base character.
fn compose(mark: char, base: char) -> Option<char> {
    COMPOSE
        .binary_search_by(|&(m, b, _)| (m, b).cmp(&(mark, base)))
        .ok()
        .map(|index| COMPOSE[index].2)
}

/// The Latin alphabet, shared by every layout below.
///
/// A layout table lists only what *differs* from this, which is why the Norwegian
/// table is thirty lines rather than sixty and why adding a third layout does not
/// mean retyping the alphabet a third time. A layout that needs a different letter
/// simply lists that position itself; its own table is searched first.
const LETTERS: [Entry; 26] = {
    use KeyCode as K;
    [
        Entry::letter(K::A, 'a', 'A'),
        Entry::letter(K::B, 'b', 'B'),
        Entry::letter(K::C, 'c', 'C'),
        Entry::letter(K::D, 'd', 'D'),
        Entry::letter(K::E, 'e', 'E'),
        Entry::letter(K::F, 'f', 'F'),
        Entry::letter(K::G, 'g', 'G'),
        Entry::letter(K::H, 'h', 'H'),
        Entry::letter(K::I, 'i', 'I'),
        Entry::letter(K::J, 'j', 'J'),
        Entry::letter(K::K, 'k', 'K'),
        Entry::letter(K::L, 'l', 'L'),
        Entry::letter(K::M, 'm', 'M'),
        Entry::letter(K::N, 'n', 'N'),
        Entry::letter(K::O, 'o', 'O'),
        Entry::letter(K::P, 'p', 'P'),
        Entry::letter(K::Q, 'q', 'Q'),
        Entry::letter(K::R, 'r', 'R'),
        Entry::letter(K::S, 's', 'S'),
        Entry::letter(K::T, 't', 'T'),
        Entry::letter(K::U, 'u', 'U'),
        Entry::letter(K::V, 'v', 'V'),
        Entry::letter(K::W, 'w', 'W'),
        Entry::letter(K::X, 'x', 'X'),
        Entry::letter(K::Y, 'y', 'Y'),
        Entry::letter(K::Z, 'z', 'Z'),
    ]
};

const US_ENTRIES: [Entry; 22] = {
    use KeyCode as K;
    [
        Entry::pair(K::Digit1, '1', '!'),
        Entry::pair(K::Digit2, '2', '@'),
        Entry::pair(K::Digit3, '3', '#'),
        Entry::pair(K::Digit4, '4', '$'),
        Entry::pair(K::Digit5, '5', '%'),
        Entry::pair(K::Digit6, '6', '^'),
        Entry::pair(K::Digit7, '7', '&'),
        Entry::pair(K::Digit8, '8', '*'),
        Entry::pair(K::Digit9, '9', '('),
        Entry::pair(K::Digit0, '0', ')'),
        Entry::pair(K::Minus, '-', '_'),
        Entry::pair(K::Equal, '=', '+'),
        Entry::pair(K::BracketLeft, '[', '{'),
        Entry::pair(K::BracketRight, ']', '}'),
        Entry::pair(K::Backslash, '\\', '|'),
        Entry::pair(K::Semicolon, ';', ':'),
        Entry::pair(K::Quote, '\'', '"'),
        Entry::pair(K::Backquote, '`', '~'),
        Entry::pair(K::Comma, ',', '<'),
        Entry::pair(K::Period, '.', '>'),
        Entry::pair(K::Slash, '/', '?'),
        // ANSI keyboards have no 102nd key; ISO ones running a US layout put a
        // second backslash there, which is what xkb does too.
        Entry::pair(K::IntlBackslash, '\\', '|'),
    ]
};

/// US QWERTY. No dead keys, no third level.
pub static US: Layout = Layout {
    name: "us",
    entries: &US_ENTRIES,
    decimal_separator: '.',
};

const NORWEGIAN_ENTRIES: [Entry; 24] = {
    use KeyCode as K;
    [
        Entry::pair(K::Backquote, '|', '\u{00a7}'),
        Entry::pair(K::Digit1, '1', '!'),
        Entry::triple(K::Digit2, '2', '"', '@'),
        Entry::triple(K::Digit3, '3', '#', '\u{00a3}'),
        Entry::triple(K::Digit4, '4', '\u{00a4}', '$'),
        Entry::triple(K::Digit5, '5', '%', '\u{20ac}'),
        Entry::pair(K::Digit6, '6', '&'),
        Entry::triple(K::Digit7, '7', '/', '{'),
        Entry::triple(K::Digit8, '8', '(', '['),
        Entry::triple(K::Digit9, '9', ')', ']'),
        Entry::triple(K::Digit0, '0', '=', '}'),
        Entry::triple(K::Minus, '+', '?', '\\'),
        // The acute and grave dead keys live here, which is why a Norwegian
        // keyboard can type é and à without a compose key.
        Entry {
            code: K::Equal,
            base: Output::Dead('\u{00b4}'),
            shift: Output::Dead('`'),
            altgr: Output::Char('|'),
            shift_altgr: Output::None,
        },
        Entry::letter(K::BracketLeft, '\u{00e5}', '\u{00c5}'),
        // Diaeresis, circumflex and tilde: three dead keys on one position, and
        // the reason ö, ô and ñ are reachable from a layout that has none of them.
        Entry {
            code: K::BracketRight,
            base: Output::Dead('\u{00a8}'),
            shift: Output::Dead('^'),
            altgr: Output::Dead('~'),
            shift_altgr: Output::None,
        },
        Entry::letter(K::Semicolon, '\u{00f8}', '\u{00d8}'),
        Entry::letter(K::Quote, '\u{00e6}', '\u{00c6}'),
        Entry::pair(K::Backslash, '\'', '*'),
        Entry::triple(K::IntlBackslash, '<', '>', '\\'),
        Entry::pair(K::Comma, ',', ';'),
        Entry::pair(K::Period, '.', ':'),
        Entry::pair(K::Slash, '-', '_'),
        // Two letters that carry a third level of their own.
        Entry::triple(K::E, 'e', 'E', '\u{20ac}'),
        Entry::triple(K::M, 'm', 'M', '\u{00b5}'),
    ]
};

/// Norwegian (Bokmål) QWERTY.
///
/// `æ`, `ø` and `å` sit on the US `'`, `;` and `[` positions, the third level is
/// AltGr, and the two dead-key positions carry five marks between them.
pub static NORWEGIAN: Layout = Layout {
    name: "no",
    entries: &NORWEGIAN_ENTRIES,
    decimal_separator: ',',
};

/// Every layout that ships, for a runtime lookup by name.
pub static BUILT_IN: [&Layout; 2] = [&US, &NORWEGIAN];

/// Finds a layout by its short name, as `setxkbmap` would name it.
pub fn by_name(name: &str) -> Option<&'static Layout> {
    BUILT_IN
        .iter()
        .copied()
        .find(|layout| layout.name.eq_ignore_ascii_case(name))
}
/// Every (mark, base) pair that composes, **sorted** so lookup can bisect.
///
/// Generated from Unicode's own canonical composition data rather than typed
/// out, because a hand-written table of a hundred accented letters is a list of
/// a hundred chances to be subtly wrong about one of them.
const COMPOSE: [(char, char, char); 118] = [
    // circumflex
    ('^', 'A', '\u{00c2}'), // LATIN CAPITAL LETTER A WITH CIRCUMFLEX
    ('^', 'C', '\u{0108}'), // LATIN CAPITAL LETTER C WITH CIRCUMFLEX
    ('^', 'E', '\u{00ca}'), // LATIN CAPITAL LETTER E WITH CIRCUMFLEX
    ('^', 'G', '\u{011c}'), // LATIN CAPITAL LETTER G WITH CIRCUMFLEX
    ('^', 'H', '\u{0124}'), // LATIN CAPITAL LETTER H WITH CIRCUMFLEX
    ('^', 'I', '\u{00ce}'), // LATIN CAPITAL LETTER I WITH CIRCUMFLEX
    ('^', 'J', '\u{0134}'), // LATIN CAPITAL LETTER J WITH CIRCUMFLEX
    ('^', 'O', '\u{00d4}'), // LATIN CAPITAL LETTER O WITH CIRCUMFLEX
    ('^', 'S', '\u{015c}'), // LATIN CAPITAL LETTER S WITH CIRCUMFLEX
    ('^', 'U', '\u{00db}'), // LATIN CAPITAL LETTER U WITH CIRCUMFLEX
    ('^', 'W', '\u{0174}'), // LATIN CAPITAL LETTER W WITH CIRCUMFLEX
    ('^', 'Y', '\u{0176}'), // LATIN CAPITAL LETTER Y WITH CIRCUMFLEX
    ('^', 'a', '\u{00e2}'), // LATIN SMALL LETTER A WITH CIRCUMFLEX
    ('^', 'c', '\u{0109}'), // LATIN SMALL LETTER C WITH CIRCUMFLEX
    ('^', 'e', '\u{00ea}'), // LATIN SMALL LETTER E WITH CIRCUMFLEX
    ('^', 'g', '\u{011d}'), // LATIN SMALL LETTER G WITH CIRCUMFLEX
    ('^', 'h', '\u{0125}'), // LATIN SMALL LETTER H WITH CIRCUMFLEX
    ('^', 'i', '\u{00ee}'), // LATIN SMALL LETTER I WITH CIRCUMFLEX
    ('^', 'j', '\u{0135}'), // LATIN SMALL LETTER J WITH CIRCUMFLEX
    ('^', 'o', '\u{00f4}'), // LATIN SMALL LETTER O WITH CIRCUMFLEX
    ('^', 's', '\u{015d}'), // LATIN SMALL LETTER S WITH CIRCUMFLEX
    ('^', 'u', '\u{00fb}'), // LATIN SMALL LETTER U WITH CIRCUMFLEX
    ('^', 'w', '\u{0175}'), // LATIN SMALL LETTER W WITH CIRCUMFLEX
    ('^', 'y', '\u{0177}'), // LATIN SMALL LETTER Y WITH CIRCUMFLEX
    // grave
    ('`', 'A', '\u{00c0}'), // LATIN CAPITAL LETTER A WITH GRAVE
    ('`', 'E', '\u{00c8}'), // LATIN CAPITAL LETTER E WITH GRAVE
    ('`', 'I', '\u{00cc}'), // LATIN CAPITAL LETTER I WITH GRAVE
    ('`', 'O', '\u{00d2}'), // LATIN CAPITAL LETTER O WITH GRAVE
    ('`', 'U', '\u{00d9}'), // LATIN CAPITAL LETTER U WITH GRAVE
    ('`', 'a', '\u{00e0}'), // LATIN SMALL LETTER A WITH GRAVE
    ('`', 'e', '\u{00e8}'), // LATIN SMALL LETTER E WITH GRAVE
    ('`', 'i', '\u{00ec}'), // LATIN SMALL LETTER I WITH GRAVE
    ('`', 'o', '\u{00f2}'), // LATIN SMALL LETTER O WITH GRAVE
    ('`', 'u', '\u{00f9}'), // LATIN SMALL LETTER U WITH GRAVE
    // tilde
    ('~', 'A', '\u{00c3}'), // LATIN CAPITAL LETTER A WITH TILDE
    ('~', 'I', '\u{0128}'), // LATIN CAPITAL LETTER I WITH TILDE
    ('~', 'N', '\u{00d1}'), // LATIN CAPITAL LETTER N WITH TILDE
    ('~', 'O', '\u{00d5}'), // LATIN CAPITAL LETTER O WITH TILDE
    ('~', 'U', '\u{0168}'), // LATIN CAPITAL LETTER U WITH TILDE
    ('~', 'a', '\u{00e3}'), // LATIN SMALL LETTER A WITH TILDE
    ('~', 'i', '\u{0129}'), // LATIN SMALL LETTER I WITH TILDE
    ('~', 'n', '\u{00f1}'), // LATIN SMALL LETTER N WITH TILDE
    ('~', 'o', '\u{00f5}'), // LATIN SMALL LETTER O WITH TILDE
    ('~', 'u', '\u{0169}'), // LATIN SMALL LETTER U WITH TILDE
    // diaeresis
    ('\u{00a8}', 'A', '\u{00c4}'), // LATIN CAPITAL LETTER A WITH DIAERESIS
    ('\u{00a8}', 'E', '\u{00cb}'), // LATIN CAPITAL LETTER E WITH DIAERESIS
    ('\u{00a8}', 'I', '\u{00cf}'), // LATIN CAPITAL LETTER I WITH DIAERESIS
    ('\u{00a8}', 'O', '\u{00d6}'), // LATIN CAPITAL LETTER O WITH DIAERESIS
    ('\u{00a8}', 'U', '\u{00dc}'), // LATIN CAPITAL LETTER U WITH DIAERESIS
    ('\u{00a8}', 'Y', '\u{0178}'), // LATIN CAPITAL LETTER Y WITH DIAERESIS
    ('\u{00a8}', 'a', '\u{00e4}'), // LATIN SMALL LETTER A WITH DIAERESIS
    ('\u{00a8}', 'e', '\u{00eb}'), // LATIN SMALL LETTER E WITH DIAERESIS
    ('\u{00a8}', 'i', '\u{00ef}'), // LATIN SMALL LETTER I WITH DIAERESIS
    ('\u{00a8}', 'o', '\u{00f6}'), // LATIN SMALL LETTER O WITH DIAERESIS
    ('\u{00a8}', 'u', '\u{00fc}'), // LATIN SMALL LETTER U WITH DIAERESIS
    ('\u{00a8}', 'y', '\u{00ff}'), // LATIN SMALL LETTER Y WITH DIAERESIS
    // acute
    ('\u{00b4}', 'A', '\u{00c1}'), // LATIN CAPITAL LETTER A WITH ACUTE
    ('\u{00b4}', 'C', '\u{0106}'), // LATIN CAPITAL LETTER C WITH ACUTE
    ('\u{00b4}', 'E', '\u{00c9}'), // LATIN CAPITAL LETTER E WITH ACUTE
    ('\u{00b4}', 'I', '\u{00cd}'), // LATIN CAPITAL LETTER I WITH ACUTE
    ('\u{00b4}', 'L', '\u{0139}'), // LATIN CAPITAL LETTER L WITH ACUTE
    ('\u{00b4}', 'N', '\u{0143}'), // LATIN CAPITAL LETTER N WITH ACUTE
    ('\u{00b4}', 'O', '\u{00d3}'), // LATIN CAPITAL LETTER O WITH ACUTE
    ('\u{00b4}', 'R', '\u{0154}'), // LATIN CAPITAL LETTER R WITH ACUTE
    ('\u{00b4}', 'S', '\u{015a}'), // LATIN CAPITAL LETTER S WITH ACUTE
    ('\u{00b4}', 'U', '\u{00da}'), // LATIN CAPITAL LETTER U WITH ACUTE
    ('\u{00b4}', 'Y', '\u{00dd}'), // LATIN CAPITAL LETTER Y WITH ACUTE
    ('\u{00b4}', 'Z', '\u{0179}'), // LATIN CAPITAL LETTER Z WITH ACUTE
    ('\u{00b4}', 'a', '\u{00e1}'), // LATIN SMALL LETTER A WITH ACUTE
    ('\u{00b4}', 'c', '\u{0107}'), // LATIN SMALL LETTER C WITH ACUTE
    ('\u{00b4}', 'e', '\u{00e9}'), // LATIN SMALL LETTER E WITH ACUTE
    ('\u{00b4}', 'i', '\u{00ed}'), // LATIN SMALL LETTER I WITH ACUTE
    ('\u{00b4}', 'l', '\u{013a}'), // LATIN SMALL LETTER L WITH ACUTE
    ('\u{00b4}', 'n', '\u{0144}'), // LATIN SMALL LETTER N WITH ACUTE
    ('\u{00b4}', 'o', '\u{00f3}'), // LATIN SMALL LETTER O WITH ACUTE
    ('\u{00b4}', 'r', '\u{0155}'), // LATIN SMALL LETTER R WITH ACUTE
    ('\u{00b4}', 's', '\u{015b}'), // LATIN SMALL LETTER S WITH ACUTE
    ('\u{00b4}', 'u', '\u{00fa}'), // LATIN SMALL LETTER U WITH ACUTE
    ('\u{00b4}', 'y', '\u{00fd}'), // LATIN SMALL LETTER Y WITH ACUTE
    ('\u{00b4}', 'z', '\u{017a}'), // LATIN SMALL LETTER Z WITH ACUTE
    // cedilla
    ('\u{00b8}', 'C', '\u{00c7}'), // LATIN CAPITAL LETTER C WITH CEDILLA
    ('\u{00b8}', 'G', '\u{0122}'), // LATIN CAPITAL LETTER G WITH CEDILLA
    ('\u{00b8}', 'K', '\u{0136}'), // LATIN CAPITAL LETTER K WITH CEDILLA
    ('\u{00b8}', 'L', '\u{013b}'), // LATIN CAPITAL LETTER L WITH CEDILLA
    ('\u{00b8}', 'N', '\u{0145}'), // LATIN CAPITAL LETTER N WITH CEDILLA
    ('\u{00b8}', 'R', '\u{0156}'), // LATIN CAPITAL LETTER R WITH CEDILLA
    ('\u{00b8}', 'S', '\u{015e}'), // LATIN CAPITAL LETTER S WITH CEDILLA
    ('\u{00b8}', 'T', '\u{0162}'), // LATIN CAPITAL LETTER T WITH CEDILLA
    ('\u{00b8}', 'c', '\u{00e7}'), // LATIN SMALL LETTER C WITH CEDILLA
    ('\u{00b8}', 'g', '\u{0123}'), // LATIN SMALL LETTER G WITH CEDILLA
    ('\u{00b8}', 'k', '\u{0137}'), // LATIN SMALL LETTER K WITH CEDILLA
    ('\u{00b8}', 'l', '\u{013c}'), // LATIN SMALL LETTER L WITH CEDILLA
    ('\u{00b8}', 'n', '\u{0146}'), // LATIN SMALL LETTER N WITH CEDILLA
    ('\u{00b8}', 'r', '\u{0157}'), // LATIN SMALL LETTER R WITH CEDILLA
    ('\u{00b8}', 's', '\u{015f}'), // LATIN SMALL LETTER S WITH CEDILLA
    ('\u{00b8}', 't', '\u{0163}'), // LATIN SMALL LETTER T WITH CEDILLA
    // caron
    ('\u{02c7}', 'C', '\u{010c}'), // LATIN CAPITAL LETTER C WITH CARON
    ('\u{02c7}', 'D', '\u{010e}'), // LATIN CAPITAL LETTER D WITH CARON
    ('\u{02c7}', 'E', '\u{011a}'), // LATIN CAPITAL LETTER E WITH CARON
    ('\u{02c7}', 'L', '\u{013d}'), // LATIN CAPITAL LETTER L WITH CARON
    ('\u{02c7}', 'N', '\u{0147}'), // LATIN CAPITAL LETTER N WITH CARON
    ('\u{02c7}', 'R', '\u{0158}'), // LATIN CAPITAL LETTER R WITH CARON
    ('\u{02c7}', 'S', '\u{0160}'), // LATIN CAPITAL LETTER S WITH CARON
    ('\u{02c7}', 'T', '\u{0164}'), // LATIN CAPITAL LETTER T WITH CARON
    ('\u{02c7}', 'Z', '\u{017d}'), // LATIN CAPITAL LETTER Z WITH CARON
    ('\u{02c7}', 'c', '\u{010d}'), // LATIN SMALL LETTER C WITH CARON
    ('\u{02c7}', 'd', '\u{010f}'), // LATIN SMALL LETTER D WITH CARON
    ('\u{02c7}', 'e', '\u{011b}'), // LATIN SMALL LETTER E WITH CARON
    ('\u{02c7}', 'l', '\u{013e}'), // LATIN SMALL LETTER L WITH CARON
    ('\u{02c7}', 'n', '\u{0148}'), // LATIN SMALL LETTER N WITH CARON
    ('\u{02c7}', 'r', '\u{0159}'), // LATIN SMALL LETTER R WITH CARON
    ('\u{02c7}', 's', '\u{0161}'), // LATIN SMALL LETTER S WITH CARON
    ('\u{02c7}', 't', '\u{0165}'), // LATIN SMALL LETTER T WITH CARON
    ('\u{02c7}', 'z', '\u{017e}'), // LATIN SMALL LETTER Z WITH CARON
    // ring above
    ('\u{02da}', 'A', '\u{00c5}'), // LATIN CAPITAL LETTER A WITH RING ABOVE
    ('\u{02da}', 'U', '\u{016e}'), // LATIN CAPITAL LETTER U WITH RING ABOVE
    ('\u{02da}', 'a', '\u{00e5}'), // LATIN SMALL LETTER A WITH RING ABOVE
    ('\u{02da}', 'u', '\u{016f}'), // LATIN SMALL LETTER U WITH RING ABOVE
];

/// Where a layout choice came from, for logging what a panel actually picked up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutSource {
    /// The `DENISE_KEYMAP` environment variable.
    Denise,
    /// `XKB_DEFAULT_LAYOUT`, as Wayland compositors use.
    Xkb,
    /// A system configuration file, named here so a wrong guess is traceable.
    File(&'static str),
    /// Nothing said, so US.
    Default,
}

impl core::fmt::Display for LayoutSource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LayoutSource::Denise => f.write_str("DENISE_KEYMAP"),
            LayoutSource::Xkb => f.write_str("XKB_DEFAULT_LAYOUT"),
            LayoutSource::File(path) => write!(f, "{path}"),
            LayoutSource::Default => f.write_str("default"),
        }
    }
}

/// Configuration files that name a console or X keyboard layout, and the key to
/// look for in each. In priority order.
const SYSTEM_FILES: [(&str, &str); 4] = [
    // systemd.
    ("/etc/vconsole.conf", "KEYMAP"),
    // Debian and Raspberry Pi OS.
    ("/etc/default/keyboard", "XKBLAYOUT"),
    // Alpine and other OpenRC systems, whose value is a path to a keymap file.
    ("/etc/conf.d/loadkmap", "KEYMAP"),
    // Void, and some minimal images.
    ("/etc/rc.conf", "KEYMAP"),
];

/// Reduces whatever a system wrote down to a layout name.
///
/// Console keymaps are named for files — `no-latin1`, `/etc/keymap/no.bmap.gz`,
/// `uk.map.gz` — so this takes the basename, drops the extensions, and then drops
/// any variant suffix if the full name matches nothing.
pub fn normalise_name(raw: &str) -> &str {
    let raw = raw.trim().trim_matches(['"', '\'']);
    let base = raw.rsplit('/').next().unwrap_or(raw);
    let stem = base.split('.').next().unwrap_or(base);
    if by_name(stem).is_some() {
        return stem;
    }
    stem.split('-').next().unwrap_or(stem)
}

/// Finds the layout this system is configured for.
///
/// Reads, in order: `DENISE_KEYMAP`, `XKB_DEFAULT_LAYOUT`, and the console
/// keyboard configuration files distributions actually use. Falls back to US.
///
/// # What this does and does not do
///
/// It reads the system's *choice of layout*, not the layout itself. The layout
/// still has to be one Denise has a table for; a system configured for `fr` on a
/// build with only `us` and `no` gets US and says so through the returned
/// [`LayoutSource`], rather than silently typing the wrong thing.
///
/// Reading the layout *data* would mean the kernel's own keymap, through
/// `KDGKBENT` on a VT — which needs `/dev/tty0`, which is `root:root` mode 600 on
/// every distribution checked. Denise otherwise runs unprivileged, needing only
/// the `video` and `input` groups, and giving that up to read a keymap is a poor
/// trade. Adding a layout table is about thirty lines; needing root is forever.
pub fn from_system() -> (&'static Layout, LayoutSource) {
    for (variable, source) in [
        ("DENISE_KEYMAP", LayoutSource::Denise),
        ("XKB_DEFAULT_LAYOUT", LayoutSource::Xkb),
    ] {
        if let Ok(value) = std::env::var(variable)
            && let Some(layout) = by_name(normalise_name(&value))
        {
            return (layout, source);
        }
    }

    for (path, key) in SYSTEM_FILES {
        let Ok(contents) = std::fs::read_to_string(path) else {
            continue;
        };
        if let Some(value) = value_of(&contents, key)
            && let Some(layout) = by_name(normalise_name(value))
        {
            return (layout, LayoutSource::File(path));
        }
    }

    (&US, LayoutSource::Default)
}

/// Finds `KEY=value` in a shell-style configuration file, ignoring comments.
fn value_of<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    contents.lines().find_map(|line| {
        let line = line.trim();
        if line.starts_with('#') {
            return None;
        }
        let (name, value) = line.split_once('=')?;
        (name.trim() == key).then(|| value.trim())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Types a sequence of positions and collects every character produced.
    fn type_keys(composer: &mut Composer, keys: &[(KeyCode, Modifiers)]) -> String {
        let mut out = String::new();
        for &(code, modifiers) in keys {
            // A real translator reports the modifier's own transition first; the
            // composer has to see AltGr go down before the key it modifies.
            if modifiers.contains(Modifiers::ALT) {
                composer.feed(KeyCode::AltRight, ElementState::Down, Modifiers::ALT);
            }
            let composed = composer.feed(code, ElementState::Down, modifiers);
            out.extend(composed.as_slice());
            composer.feed(code, ElementState::Up, modifiers);
            if modifiers.contains(Modifiers::ALT) {
                composer.feed(KeyCode::AltRight, ElementState::Up, Modifiers::NONE);
            }
        }
        out
    }

    fn plain(keys: &[KeyCode]) -> Vec<(KeyCode, Modifiers)> {
        keys.iter().map(|&k| (k, Modifiers::NONE)).collect()
    }

    #[test]
    fn us_types_ascii() {
        let mut c = Composer::new(&US);
        assert_eq!(
            type_keys(&mut c, &plain(&[KeyCode::H, KeyCode::I, KeyCode::Digit1])),
            "hi1"
        );
        assert_eq!(
            type_keys(
                &mut c,
                &[
                    (KeyCode::H, Modifiers::SHIFT),
                    (KeyCode::Digit1, Modifiers::SHIFT),
                ]
            ),
            "H!"
        );
    }

    #[test]
    fn norwegian_types_the_three_letters_it_exists_for() {
        let mut c = Composer::new(&NORWEGIAN);
        // æ, ø and å sit where a US layout has ' ; [ — the whole reason a
        // position is not a character.
        assert_eq!(
            type_keys(
                &mut c,
                &plain(&[KeyCode::Quote, KeyCode::Semicolon, KeyCode::BracketLeft])
            ),
            "æøå"
        );
        assert_eq!(
            type_keys(
                &mut c,
                &[
                    (KeyCode::Quote, Modifiers::SHIFT),
                    (KeyCode::Semicolon, Modifiers::SHIFT),
                    (KeyCode::BracketLeft, Modifiers::SHIFT),
                ]
            ),
            "ÆØÅ"
        );
    }

    #[test]
    fn the_same_positions_type_ascii_on_a_us_layout() {
        let mut c = Composer::new(&US);
        assert_eq!(
            type_keys(
                &mut c,
                &plain(&[KeyCode::Quote, KeyCode::Semicolon, KeyCode::BracketLeft])
            ),
            "';["
        );
    }

    #[test]
    fn dead_keys_compose() {
        let mut c = Composer::new(&NORWEGIAN);
        // ¨ then o is the sequence the milestone is actually about.
        assert_eq!(
            type_keys(&mut c, &plain(&[KeyCode::BracketRight, KeyCode::O])),
            "ö"
        );
        // Acute and grave live on the other dead position.
        assert_eq!(
            type_keys(&mut c, &plain(&[KeyCode::Equal, KeyCode::E])),
            "é"
        );
        assert_eq!(
            type_keys(
                &mut c,
                &[
                    (KeyCode::Equal, Modifiers::SHIFT),
                    (KeyCode::A, Modifiers::NONE)
                ]
            ),
            "à"
        );
        // Circumflex is the shifted diaeresis key; tilde is its third level.
        assert_eq!(
            type_keys(
                &mut c,
                &[
                    (KeyCode::BracketRight, Modifiers::SHIFT),
                    (KeyCode::O, Modifiers::NONE)
                ]
            ),
            "ô"
        );
        assert_eq!(
            type_keys(
                &mut c,
                &[
                    (KeyCode::BracketRight, Modifiers::ALT),
                    (KeyCode::N, Modifiers::NONE)
                ]
            ),
            "ñ"
        );
    }

    #[test]
    fn a_dead_key_produces_nothing_until_it_is_resolved() {
        let mut c = Composer::new(&NORWEGIAN);
        let composed = c.feed(KeyCode::BracketRight, ElementState::Down, Modifiers::NONE);
        assert!(composed.is_empty(), "a dead key must not type anything yet");
        assert_eq!(c.pending_dead(), Some('¨'));
    }

    #[test]
    fn a_dead_key_twice_types_the_mark_itself() {
        let mut c = Composer::new(&NORWEGIAN);
        assert_eq!(
            type_keys(
                &mut c,
                &plain(&[KeyCode::BracketRight, KeyCode::BracketRight])
            ),
            "¨"
        );
        assert_eq!(c.pending_dead(), None);
    }

    #[test]
    fn space_after_a_dead_key_types_the_bare_mark() {
        let mut c = Composer::new(&NORWEGIAN);
        assert_eq!(
            type_keys(&mut c, &plain(&[KeyCode::BracketRight, KeyCode::Space])),
            "¨"
        );
    }

    #[test]
    fn a_dead_key_that_cannot_combine_emits_both() {
        let mut c = Composer::new(&NORWEGIAN);
        // Dropping the mark silently would be worse: the user typed it, and there
        // is no undo for a character that never appeared.
        assert_eq!(
            type_keys(&mut c, &plain(&[KeyCode::BracketRight, KeyCode::Q])),
            "¨q"
        );
    }

    #[test]
    fn a_key_that_types_nothing_cancels_a_pending_mark() {
        let mut c = Composer::new(&NORWEGIAN);
        c.feed(KeyCode::BracketRight, ElementState::Down, Modifiers::NONE);
        assert_eq!(c.pending_dead(), Some('¨'));
        c.feed(KeyCode::Escape, ElementState::Down, Modifiers::NONE);
        assert_eq!(
            c.pending_dead(),
            None,
            "Escape must not leave a latch behind"
        );
        assert_eq!(type_keys(&mut c, &plain(&[KeyCode::O])), "o");
    }

    #[test]
    fn switching_layouts_abandons_a_half_typed_composition() {
        let mut c = Composer::new(&NORWEGIAN);
        c.feed(KeyCode::BracketRight, ElementState::Down, Modifiers::NONE);
        c.set_layout(&US);
        assert_eq!(c.pending_dead(), None);
    }

    #[test]
    fn the_third_level_needs_the_right_alt_key() {
        let mut c = Composer::new(&NORWEGIAN);
        assert_eq!(type_keys(&mut c, &[(KeyCode::Digit2, Modifiers::ALT)]), "@");
        assert_eq!(type_keys(&mut c, &[(KeyCode::Digit7, Modifiers::ALT)]), "{");
        assert_eq!(type_keys(&mut c, &[(KeyCode::E, Modifiers::ALT)]), "€");

        // The *left* Alt is a binding modifier, not a level. It never types.
        let mut c = Composer::new(&NORWEGIAN);
        let composed = c.feed(KeyCode::Digit2, ElementState::Down, Modifiers::ALT);
        assert!(
            composed.is_empty(),
            "left Alt must not reach the third level"
        );
    }

    #[test]
    fn altgr_reaches_the_third_level_even_reported_as_ctrl_plus_alt() {
        // Plenty of keyboards and firmwares send Ctrl alongside AltGr. A rule
        // that let Ctrl veto text would disable the entire third level on those,
        // silently, and only on the hardware nobody tested with.
        let mut c = Composer::new(&NORWEGIAN);
        c.feed(KeyCode::ControlLeft, ElementState::Down, Modifiers::CTRL);
        c.feed(
            KeyCode::AltRight,
            ElementState::Down,
            Modifiers::CTRL | Modifiers::ALT,
        );
        let composed = c.feed(
            KeyCode::Digit2,
            ElementState::Down,
            Modifiers::CTRL | Modifiers::ALT,
        );
        assert_eq!(composed.as_slice(), ['@']);

        // Releasing AltGr puts Ctrl back in charge, and Ctrl types nothing.
        c.feed(KeyCode::AltRight, ElementState::Up, Modifiers::CTRL);
        let composed = c.feed(KeyCode::Digit2, ElementState::Down, Modifiers::CTRL);
        assert!(composed.is_empty(), "Ctrl+2 is a binding, not an at sign");
    }

    #[test]
    fn control_chords_type_nothing() {
        let mut c = Composer::new(&US);
        for modifier in [Modifiers::CTRL, Modifiers::SUPER] {
            let composed = c.feed(KeyCode::C, ElementState::Down, modifier);
            assert!(composed.is_empty(), "{modifier:?} + C must not type a c");
        }
    }

    #[test]
    fn control_and_enter_and_backspace_are_never_text() {
        let mut c = Composer::new(&NORWEGIAN);
        for code in [
            KeyCode::Enter,
            KeyCode::Tab,
            KeyCode::Backspace,
            KeyCode::Delete,
            KeyCode::ArrowLeft,
            KeyCode::F1,
        ] {
            let composed = c.feed(code, ElementState::Down, Modifiers::NONE);
            assert!(composed.is_empty(), "{code:?} must not produce text");
        }
    }

    #[test]
    fn caps_lock_shifts_letters_and_leaves_the_digit_row_alone() {
        let mut c = Composer::new(&NORWEGIAN);
        c.feed(KeyCode::CapsLock, ElementState::Down, Modifiers::NONE);
        assert!(c.caps_lock());
        assert_eq!(
            type_keys(
                &mut c,
                &plain(&[KeyCode::A, KeyCode::Quote, KeyCode::Digit1])
            ),
            "AÆ1",
            "caps lock must reach æøå but not turn 1 into !"
        );
        // Shift with caps lock on gives lower case again.
        assert_eq!(type_keys(&mut c, &[(KeyCode::A, Modifiers::SHIFT)]), "a");
        c.feed(KeyCode::CapsLock, ElementState::Down, Modifiers::NONE);
        assert!(!c.caps_lock());
    }

    #[test]
    fn the_numpad_follows_num_lock_and_the_layout() {
        let mut us = Composer::new(&US);
        assert_eq!(
            type_keys(&mut us, &plain(&[KeyCode::Numpad4, KeyCode::NumpadDecimal])),
            "4."
        );
        let mut no = Composer::new(&NORWEGIAN);
        assert_eq!(
            type_keys(&mut no, &plain(&[KeyCode::Numpad4, KeyCode::NumpadDecimal])),
            "4,",
            "a European numpad types a decimal comma"
        );

        no.feed(KeyCode::NumLock, ElementState::Down, Modifiers::NONE);
        let composed = no.feed(KeyCode::Numpad4, ElementState::Down, Modifiers::NONE);
        assert!(
            composed.is_empty(),
            "with num lock off the numpad is arrows, not digits"
        );
    }

    #[test]
    fn key_release_types_nothing() {
        let mut c = Composer::new(&US);
        let composed = c.feed(KeyCode::A, ElementState::Up, Modifiers::NONE);
        assert!(composed.is_empty(), "a key types on the way down, once");
    }

    #[test]
    fn the_compose_table_is_sorted_and_free_of_duplicates() {
        assert!(
            COMPOSE
                .windows(2)
                .all(|w| (w[0].0, w[0].1) < (w[1].0, w[1].1)),
            "lookup bisects, so an unsorted table would silently miss entries"
        );
    }

    #[test]
    fn composition_matches_unicode() {
        // Spot checks across every mark, against what NFC would produce. The table
        // is generated from Unicode's data; this is the assertion that the
        // generation was not quietly wrong.
        for (mark, base, expected) in [
            ('´', 'e', 'é'),
            ('`', 'a', 'à'),
            ('¨', 'u', 'ü'),
            ('^', 'i', 'î'),
            ('~', 'n', 'ñ'),
            ('\u{02da}', 'a', 'å'),
            ('¸', 'c', 'ç'),
            ('\u{02c7}', 's', 'š'),
        ] {
            assert_eq!(compose(mark, base), Some(expected), "{mark}{base}");
        }
        assert_eq!(compose('¨', 'q'), None);
        assert_eq!(compose('!', 'a'), None);
    }

    #[test]
    fn no_layout_lists_a_position_twice() {
        for layout in BUILT_IN {
            for (i, entry) in layout.entries.iter().enumerate() {
                assert!(
                    !layout.entries[..i].iter().any(|e| e.code == entry.code),
                    "{} lists {:?} twice; the first would silently win",
                    layout.name,
                    entry.code
                );
            }
        }
    }

    #[test]
    fn every_layout_can_type_the_whole_alphabet_and_the_digits() {
        for layout in BUILT_IN {
            let mut c = Composer::new(layout);
            let letters = type_keys(
                &mut c,
                &plain(&[
                    KeyCode::A,
                    KeyCode::B,
                    KeyCode::C,
                    KeyCode::X,
                    KeyCode::Y,
                    KeyCode::Z,
                ]),
            );
            assert_eq!(letters, "abcxyz", "{}", layout.name);
            let digits = type_keys(
                &mut c,
                &plain(&[KeyCode::Digit0, KeyCode::Digit5, KeyCode::Digit9]),
            );
            assert_eq!(digits, "059", "{}", layout.name);
        }
    }

    #[test]
    fn keymap_names_are_reduced_to_something_findable() {
        // The shapes real systems write down. Alpine names a gzipped file path,
        // Debian a bare code, systemd a console keymap with a variant suffix.
        assert_eq!(normalise_name("/etc/keymap/no.bmap.gz"), "no");
        assert_eq!(normalise_name("\"no\""), "no");
        assert_eq!(normalise_name("no-latin1"), "no");
        assert_eq!(normalise_name("us"), "us");
        assert_eq!(normalise_name("/usr/share/keymaps/xkb/us.map.gz"), "us");
        // A layout that is not shipped reduces to something that still misses,
        // rather than to a near-match that would type the wrong characters.
        assert!(by_name(normalise_name("fr-bepo")).is_none());
    }

    #[test]
    fn a_configuration_file_is_parsed_the_way_a_shell_would() {
        let alpine = "# Absolut path to the keymap.\n                      #KEYMAP=\"/usr/share/keymaps/xkb/us.map.gz\"\n                      KEYMAP=/etc/keymap/no.bmap.gz\n";
        let value = value_of(alpine, "KEYMAP").expect("a value");
        assert_eq!(
            normalise_name(value),
            "no",
            "the commented-out line must not win"
        );

        assert_eq!(value_of("XKBLAYOUT=\"gb\"\n", "XKBLAYOUT"), Some("\"gb\""));
        assert_eq!(value_of("# nothing here\n", "KEYMAP"), None);
    }

    #[test]
    fn layouts_are_findable_by_name() {
        assert!(core::ptr::eq(by_name("no").expect("no"), &NORWEGIAN));
        assert!(core::ptr::eq(by_name("US").expect("us"), &US));
        assert_eq!(by_name("dvorak").map(|l| l.name), None);
    }
}
