//! Where the keys are: a QWERTY-shaped grid of positions.
//!
//! Positions, not characters. What each one *types* is the layout's answer and
//! is read at build time, which is why a layout switch relabels the keys rather
//! than rebuilding them — `ø` and `;` live on [`KeyCode::Semicolon`] on
//! Norwegian and US respectively, and the key does not move.
//!
//! # The shape, and why this one
//!
//! A compact physical keyboard — a Chromebook, near enough: no numpad, no
//! function row, Backspace at the top right, Enter to the right of the home
//! row, Shift at the left of the bottom one, and a key of its own where Caps
//! Lock would be. Chosen over a phone keyboard because of what this is for: a
//! panel somebody stands in front of, typing addresses into a browser and
//! filling in a form. Digits belong on screen rather than behind a `123` page,
//! and Tab is how you get from one field to the next.
//!
//! It is also **shorter than what it replaced**. The first version had six
//! rows, because Backspace, Enter, Shift and the layout key were all crowded
//! into a modifier row of their own; putting each where a hand reaches for it
//! empties that row entirely. Five rows instead of six is 276 logical pixels
//! instead of 330 — 54 back, which on an 800x480 panel is eleven per cent of
//! the screen returned to the application.
//!
//! Legends are ASCII words rather than the obvious symbols, and that is not
//! taste. `denise-render`'s built-in face covers ASCII plus a handful of Nordic
//! letters, and a stock Alpine root has no fonts installed at all — so `⌫`,
//! `⇧`, `⏎` and `←` draw as tofu on precisely the machine least able to spare a
//! key nobody can read.

use denise::KeyCode;

use crate::LAYOUT_KEY as LAYOUT;

/// One key in the grid.
#[derive(Clone, Copy, Debug)]
pub struct Key {
    /// The position this key stands for.
    pub code: KeyCode,
    /// A fixed legend, for keys whose meaning is not a character the layout
    /// knows about. `None` means "ask the layout".
    pub legend: Option<&'static str>,
    /// Width, in half-widths of an ordinary letter key.
    ///
    /// Halves rather than whole keys because the useful sizes are not multiples
    /// of a letter: Tab and Shift want one and a half, and a row that could only
    /// count in whole letters would have to round one of them wrong.
    pub units: i32,
    /// Whether holding it keeps sending it.
    ///
    /// True for Backspace and nothing else, which is what a phone does and what
    /// stops a slow finger typing `aaaaaa`. Holding a *letter* is a gesture with
    /// a different meaning — the alternates a layout offers for it — and is not
    /// this.
    pub repeats: bool,
}

/// One letter's worth of width, in the units above.
const LETTER: i32 = 2;

impl Key {
    /// A key lettered by the layout, one letter wide.
    const fn new(code: KeyCode) -> Self {
        Self {
            code,
            legend: None,
            units: LETTER,
            repeats: false,
        }
    }

    /// A key with a legend of its own, `units` half-letters wide.
    const fn wide(code: KeyCode, legend: &'static str, units: i32) -> Self {
        Self {
            code,
            legend: Some(legend),
            units,
            repeats: false,
        }
    }

    /// The same key, repeating while it is held.
    const fn repeating(mut self) -> Self {
        self.repeats = true;
        self
    }
}

/// One row of keys.
#[derive(Clone, Copy, Debug)]
pub struct Row {
    /// Left to right.
    pub keys: &'static [Key],
}

// Fourteen columns, which is what both a Chromebook and an iPad settle on for
// a keyboard of this shape. The width is not decoration: `Minus`, `Equal`,
// `Backquote`, `BracketLeft`, `BracketRight` and `Backslash` are where a
// Norwegian layout keeps `+`, `?`, the acute and grave dead keys, `å`, the
// diaeresis, and `'`. A narrower grid cannot type `å` at all — one of the three
// letters the layout exists for.
const DIGITS: [Key; 14] = [
    Key::new(KeyCode::Backquote),
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
    Key::new(KeyCode::Minus),
    Key::new(KeyCode::Equal),
    // Where every keyboard puts it, and where a hand reaching to correct a typo
    // already goes.
    Key::wide(KeyCode::Backspace, "back", LETTER * 2).repeating(),
];

// Tab opens the second row, as it does on a real one. It is here because a form
// is the main thing this keyboard is for and Tab is how you cross it — the tree
// already moves focus on Tab, and until now the keyboard had no key to send it
// with. `BracketLeft` carries å on Norwegian and `BracketRight` the diaeresis
// that makes ö and ñ reachable.
const TOP: [Key; 14] = [
    Key::wide(KeyCode::Tab, "tab", LETTER * 3 / 2),
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
    Key::new(KeyCode::BracketLeft),
    Key::new(KeyCode::BracketRight),
    Key::new(KeyCode::Backslash),
];

// The layout key takes the slot Caps Lock has on the keyboards this is shaped
// after. Caps itself does not need one: Shift here cycles off, once, locked, so
// a separate Caps key would be a second way to reach a state that already has
// one. `Semicolon` and `Quote` carry ø and æ on Norwegian and `;` and `'` on
// US — the position is the same, only the legend moves — and Enter closes the
// row they are on, which is where a hand looks for it.
const HOME: [Key; 13] = [
    Key {
        code: LAYOUT,
        legend: None,
        units: LETTER * 2,
        repeats: false,
    },
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
    Key::wide(KeyCode::Enter, "enter", LETTER * 2),
];

// Shift at both ends, as on both references. They are the same position and do
// the same thing; having two is what lets either hand reach one.
const BOTTOM: [Key; 12] = [
    Key::wide(KeyCode::ShiftLeft, "shift", LETTER * 5 / 2),
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
    Key::wide(KeyCode::ShiftLeft, "shift", LETTER * 5 / 2),
];

// Escape is how a panel with no other keyboard puts this one away, and the
// arrows are so that correcting the middle of an address does not mean retyping
// its tail. `<-` and `->` rather than `<` and `>`, which on a Norwegian layout
// are characters a key can type — and rather than the references' `◀ ▶`, which
// draw as tofu on a stock Alpine with no fonts installed.
const SPACE_ROW: [Key; 5] = [
    Key::wide(KeyCode::Escape, "esc", LETTER * 2),
    // The third level, whose legend comes from the keyboard's state rather than
    // the layout: the key says what it will do next, not what it types.
    Key::wide(KeyCode::AltRight, "alt", LETTER * 2),
    Key::wide(KeyCode::Space, " ", LETTER * 9),
    Key::wide(KeyCode::ArrowLeft, "<-", LETTER * 3 / 2),
    Key::wide(KeyCode::ArrowRight, "->", LETTER * 3 / 2),
];

/// The grid, top row first.
pub static ROWS: [Row; 5] = [
    Row { keys: &DIGITS },
    Row { keys: &TOP },
    Row { keys: &HOME },
    Row { keys: &BOTTOM },
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
