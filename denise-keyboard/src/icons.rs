//! The keys' pictures, drawn rather than looked up in a font.
//!
//! Every shape here is a filled polygon on a hundred-square box — see
//! [`denise_render::icon`] for why there are no strokes and why an outline is
//! made by knocking a hole in a fill.
//!
//! # Which keys get one
//!
//! The ones whose legend is a *name*: Backspace, Tab, Enter, the two cursor
//! keys, the layout key and Escape. A picture is better than a word for those
//! on any panel, and until now whether it could be drawn at all depended on the
//! font.
//!
//! The ones whose legend carries **state** keep their words, and that is not a
//! gap to be filled later. `shift` becomes `SHIFT`, `caps` becomes `CAPS`,
//! `ctrl` becomes `CTRL`: one glyph cannot say which of two states it is in,
//! and a keyboard whose Shift key looks identical armed and unarmed is worse
//! than one that spells it out.
//!
//! The layout key is the one that is *both*. A globe says what it is for; it
//! cannot say which of three layouts is live, and on a panel with no other
//! keyboard that is the question only this key can answer. So it wears the
//! globe and keeps the name in its corner — a `Button` draws both.
//!
//! Escape earns a picture on this keyboard for a reason it would not have
//! anywhere else: here the key's job is to put the keyboard away, and a
//! keyboard going downwards is what every phone draws for that. The `⎋` glyph
//! it does *not* get is both the correct symbol and one almost nobody reads —
//! and no font here has it either, which is a fair sign of how often it is
//! wanted.

use denise_render::icon::{Icon, Shape};

/// Backspace: a tag pointing left with a cross knocked out of it.
pub static BACKSPACE: Icon = Icon::new(&[
    Shape::fore(&[(10, 50), (36, 22), (90, 22), (90, 78), (36, 78)]),
    // The cross, in two parallelograms. Knocked out rather than drawn over,
    // because the key's own colour is what shows through.
    Shape::back(&[(55, 37), (73, 55), (67, 61), (49, 43)]),
    Shape::back(&[(67, 37), (73, 43), (55, 61), (49, 55)]),
]);

/// Tab: an arrow into a bar.
pub static TAB: Icon = Icon::new(&[
    Shape::fore(&[(14, 44), (58, 44), (58, 56), (14, 56)]),
    Shape::fore(&[(56, 30), (80, 50), (56, 70)]),
    Shape::fore(&[(84, 26), (94, 26), (94, 74), (84, 74)]),
]);

/// Enter: a shaft turning up on the right, with the head on the left.
pub static ENTER: Icon = Icon::new(&[
    Shape::fore(&[(16, 62), (42, 42), (42, 82)]),
    Shape::fore(&[(38, 56), (88, 56), (88, 68), (38, 68)]),
    Shape::fore(&[(78, 20), (88, 20), (88, 68), (78, 68)]),
]);

/// The cursor keys: solid triangles, as on the keyboards this grid is shaped
/// after.
pub static ARROW_LEFT: Icon = Icon::new(&[Shape::fore(&[(30, 50), (66, 24), (66, 76)])]);

/// The right-hand one.
pub static ARROW_RIGHT: Icon = Icon::new(&[Shape::fore(&[(70, 50), (34, 24), (34, 76)])]);

/// The layout key: a globe, with the layout's own name beside it.
///
/// Twenty-two-sided circles rather than a curve primitive, because there is no
/// curve primitive — an icon here is filled polygons and nothing else. The ring
/// is the outer disc with the inner one knocked back out, which is the only way
/// to draw an outline in a rasteriser with no stroke.
///
/// Two parallels and a meridian, which is the fewest lines that read as a globe
/// rather than as a target. They stop at the inner circle and merge into the
/// ring, the way the lines on a globe meet its edge.
pub static GLOBE: Icon = Icon::new(&[
    Shape::fore(&[
        (50, 4),
        (63, 6),
        (75, 11),
        (85, 20),
        (92, 31),
        (96, 43),
        (96, 57),
        (92, 69),
        (85, 80),
        (75, 89),
        (63, 94),
        (50, 96),
        (37, 94),
        (25, 89),
        (15, 80),
        (8, 69),
        (4, 57),
        (4, 43),
        (8, 31),
        (15, 20),
        (25, 11),
        (37, 6),
    ]),
    Shape::back(&[
        (40, 15),
        (31, 20),
        (23, 26),
        (17, 35),
        (14, 45),
        (14, 55),
        (17, 65),
        (23, 74),
        (31, 80),
        (40, 85),
        (50, 86),
        (60, 85),
        (69, 80),
        (77, 74),
        (83, 65),
        (86, 55),
        (86, 45),
        (83, 35),
        (77, 26),
        (69, 20),
        (60, 15),
        (50, 14),
    ]),
    // The equator, spanning the full width of the hole.
    Shape::fore(&[(14, 46), (86, 46), (86, 54), (14, 54)]),
    // The meridian, which is what stops it reading as a clock face.
    Shape::fore(&[(46, 14), (54, 14), (54, 86), (46, 86)]),
    Shape::fore(&[(22, 27), (78, 27), (78, 33), (22, 33)]),
    Shape::fore(&[(22, 67), (78, 67), (78, 73), (22, 73)]),
]);

/// Escape: a keyboard with an arrow leaving it downwards.
///
/// What the key *does* here, which is not what Escape does anywhere else: it
/// puts this keyboard away. Every phone draws that as a keyboard going down and
/// nobody has to be told what it means — which is the argument the `\u{2387}`
/// glyph loses, being both correct and unread.
///
/// The key still reports `esc` and still emits a real `Escape`, so a field that
/// cancels on Escape hears one.
pub static DISMISS: Icon = Icon::new(&[
    // Solid, with the keys knocked out of it, rather than an outline with the
    // keys drawn inside. At the size a key actually renders — around a quarter
    // of 48 logical pixels — a two-pixel outline holding two two-pixel bars is
    // mush, and the same shape inverted is not: the mass reads first and the
    // slots read as detail on it.
    Shape::fore(&[(4, 8), (96, 8), (96, 46), (4, 46)]),
    Shape::back(&[(14, 16), (86, 16), (86, 24), (14, 24)]),
    Shape::back(&[(32, 31), (68, 31), (68, 39), (32, 39)]),
    // Below, with air between: the arrow is leaving the keyboard, and a
    // triangle touching it would read as part of it.
    Shape::fore(&[(26, 60), (74, 60), (50, 94)]),
]);
