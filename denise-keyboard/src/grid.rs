//! Where the keys are: a QWERTY-shaped grid of positions.
//!
//! Positions, not characters. What each one *types* is the layout's answer and
//! is read at build time, which is why a layout switch relabels the keys rather
//! than rebuilding them — `ø` and `;` live on [`KeyCode::Semicolon`] on
//! Norwegian and US respectively, and the key does not move.
//!
//! The shape is the ISO/ANSI overlap: the positions both have, in the rows a
//! reader expects to find them in. Keys that differ between the two are not here.

use denise::KeyCode;

/// One key in the grid.
#[derive(Clone, Copy, Debug)]
pub struct Key {
    /// The position this key stands for.
    pub code: KeyCode,
    /// A fixed legend, for keys whose meaning is not a character the layout
    /// knows about. `None` means "ask the layout".
    pub legend: Option<&'static str>,
    /// Width, in units of the narrowest key in the row.
    pub units: i32,
}

impl Key {
    /// A key lettered by the layout.
    const fn new(code: KeyCode) -> Self {
        Self {
            code,
            legend: None,
            units: 1,
        }
    }

    /// A key with a legend of its own, `units` wide.
    const fn wide(code: KeyCode, legend: &'static str, units: i32) -> Self {
        Self {
            code,
            legend: Some(legend),
            units,
        }
    }
}

/// One row of keys.
#[derive(Clone, Copy, Debug)]
pub struct Row {
    /// Left to right.
    pub keys: &'static [Key],
}

const DIGITS: [Key; 10] = [
    Key::new(KeyCode::Digit1),
    Key::new(KeyCode::Digit2),
    Key::new(KeyCode::Digit3),
    Key::new(KeyCode::Digit4),
    Key::new(KeyCode::Digit5),
    Key::new(KeyCode::Digit6),
    Key::new(KeyCode::Digit7),
    Key::new(KeyCode::Digit8),
    Key::new(KeyCode::Digit9),
    Key::new(KeyCode::Digit0),
];

const TOP: [Key; 10] = [
    Key::new(KeyCode::Q),
    Key::new(KeyCode::W),
    Key::new(KeyCode::E),
    Key::new(KeyCode::R),
    Key::new(KeyCode::T),
    Key::new(KeyCode::Y),
    Key::new(KeyCode::U),
    Key::new(KeyCode::I),
    Key::new(KeyCode::O),
    Key::new(KeyCode::P),
];

// `Semicolon` and `Quote` carry ø and æ on a Norwegian layout and ; and ' on a
// US one. The position is the same; only the legend moves.
const HOME: [Key; 11] = [
    Key::new(KeyCode::A),
    Key::new(KeyCode::S),
    Key::new(KeyCode::D),
    Key::new(KeyCode::F),
    Key::new(KeyCode::G),
    Key::new(KeyCode::H),
    Key::new(KeyCode::J),
    Key::new(KeyCode::K),
    Key::new(KeyCode::L),
    Key::new(KeyCode::Semicolon),
    Key::new(KeyCode::Quote),
];

// `BracketRight` is the dead-key position on Norwegian — the one that makes ö
// and ñ reachable — and `]` on US. Worth a key for the same reason as above.
const BOTTOM: [Key; 11] = [
    Key::new(KeyCode::Z),
    Key::new(KeyCode::X),
    Key::new(KeyCode::C),
    Key::new(KeyCode::V),
    Key::new(KeyCode::B),
    Key::new(KeyCode::N),
    Key::new(KeyCode::M),
    Key::new(KeyCode::Comma),
    Key::new(KeyCode::Period),
    Key::new(KeyCode::Slash),
    Key::new(KeyCode::BracketRight),
];

// Backspace and Enter are `Key` events and never text, so they carry legends of
// their own: the layout has nothing to say about a position that types nothing.
//
// Words rather than the obvious `⌫` and `↵`, and the reason is the panel this
// is for. `denise-render`'s built-in face covers ASCII plus a handful of Nordic
// letters, and a stock Alpine root has no fonts installed at all — so the
// symbols draw as tofu on precisely the machine least able to spare a key
// nobody can read. ASCII renders in every face there is.
const MOD_ROW: [Key; 3] = [
    // Legends for these two come from the keyboard's state rather than the
    // layout: the key says what it will do next, not what it types.
    Key::wide(KeyCode::ShiftLeft, "shift", 2),
    Key::wide(KeyCode::AltRight, "alt", 2),
    Key::wide(KeyCode::Backspace, "back", 2),
];

const SPACE_ROW: [Key; 2] = [
    Key::wide(KeyCode::Space, " ", 8),
    Key::wide(KeyCode::Enter, "enter", 2),
];

/// The grid, top row first.
pub static ROWS: [Row; 6] = [
    Row { keys: &DIGITS },
    Row { keys: &TOP },
    Row { keys: &HOME },
    Row { keys: &BOTTOM },
    Row { keys: &MOD_ROW },
    Row { keys: &SPACE_ROW },
];

/// The fixed legend for a position, if it has one.
///
/// Keys that type nothing carry their own words; everything else asks the
/// layout, and the answer changes with the shift level.
pub(crate) fn legend_of(code: KeyCode) -> Option<&'static str> {
    ROWS.iter()
        .flat_map(|row| row.keys)
        .find(|key| key.code == code)
        .and_then(|key| key.legend)
}
