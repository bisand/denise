//! The keys' pictures, drawn rather than looked up in a font.
//!
//! Every shape here is a filled polygon on a hundred-square box — see
//! [`denise_render::icon`] for why there are no strokes and why an outline is
//! made by knocking a hole in a fill.
//!
//! # Which keys get one
//!
//! The ones whose legend is a *name*: Backspace, Tab, Enter and the two cursor
//! keys. A picture is better than a word for those on any panel, and until now
//! whether it could be drawn at all depended on the font.
//!
//! The ones whose legend carries **state** keep their words, and that is not a
//! gap to be filled later. `shift` becomes `SHIFT`, `caps` becomes `CAPS`,
//! `ctrl` becomes `CTRL`: one glyph cannot say which of two states it is in,
//! and a keyboard whose Shift key looks identical armed and unarmed is worse
//! than one that spells it out. The layout key says `no` or `us`, which is the
//! same argument.
//!
//! Escape keeps its word too, for a different reason: `⎋` is the correct symbol
//! and almost nobody knows it. No font here has the glyph either, which is a
//! fair sign of how often it is wanted.

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
