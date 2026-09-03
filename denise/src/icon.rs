//! Shapes a widget can draw when the font has no glyph for them.
//!
//! # Why this exists
//!
//! A `⌫` on a Backspace key is a picture of an idea, and whether it can be
//! drawn at all currently depends on which font happens to be installed. The
//! answers differ more than one would guess: DejaVu has `⌫`, `⇥`, `⏎` and the
//! cursor triangles; a Mac's Arial has none of them and no triangle either; and
//! the face that ships with this crate has twenty-three non-ASCII glyphs of
//! which not one is either. A key that says "back" is legible everywhere and
//! looks like a compromise; a key that says `⌫` looks right and is a box on the
//! machine least able to spare one.
//!
//! An icon is drawn rather than looked up, so it is the same on every machine.
//!
//! # Filled polygons, and nothing else
//!
//! There is no path builder here and still is not one. An [`Icon`] is a short
//! list of closed polygons on a [`GRID`]-square box, scaled into whatever
//! rectangle it is asked for — which is enough for every shape a key or a
//! toolbar wants, and stops well short of a vector format this crate would have
//! to support forever.
//!
//! Strokes are absent for a reason rather than an oversight:
//! [`Painter::draw_line`](crate::Painter::draw_line) has no thickness and
//! deliberately does not, so a one-pixel outline on a 48-pixel key would be
//! invisible. A shape that reads as an outline is drawn as a filled polygon
//! with the middle knocked back out in [`Ink::Back`] — which is also how the
//! `×` inside `⌫` is made.
//!
//! # Coordinates
//!
//! Integers on a `0..=`[`GRID`] box, y downwards, scaled with integer
//! arithmetic. No floating point anywhere: this crate has neither `std` nor
//! `libm`, and every renderer downstream of it is built on that.

use crate::angle::{COORD_LIMIT, ONE};

/// The most vertices one icon polygon may have.
pub const MAX_ICON_VERTICES: usize = 32;

/// The side of the square an icon's coordinates are given on.
///
/// A hundred because it reads as a percentage and divides by enough to place
/// things on halves, quarters and fifths without fractions.
pub const GRID: i32 = 100;

/// The most polygons one icon may have.
///
/// Six is a filled shape, a hole and room to spare. An icon needing more than
/// this is a drawing, and a drawing belongs in a `denise-image` decoder.
pub const MAX_SHAPES: usize = 6;

/// Which of the two colours a shape is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    /// The content colour — the same one the label would be drawn in.
    Fore,
    /// The colour behind the icon, for knocking a hole out of a filled shape.
    ///
    /// The only way to draw an outline here, since the filler has no stroke and
    /// no even-odd rule. It is why `⌫` can have an `×` in it.
    Back,
}

/// One closed polygon of an icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    /// Vertices on the `0..=`[`GRID`] box, y downwards, in order round the
    /// outline. Three at least; [`MAX_VERTICES`](crate::MAX_ICON_VERTICES) at most.
    pub points: &'static [(i16, i16)],
    /// Which colour it is drawn in.
    pub ink: Ink,
}

impl Shape {
    /// A shape in the content colour.
    pub const fn fore(points: &'static [(i16, i16)]) -> Self {
        Self {
            points,
            ink: Ink::Fore,
        }
    }

    /// A shape knocked back out in the background colour.
    pub const fn back(points: &'static [(i16, i16)]) -> Self {
        Self {
            points,
            ink: Ink::Back,
        }
    }
}

/// A small drawing, in shapes rather than glyphs.
///
/// Order matters: shapes are drawn front to back in the order given, so a hole
/// comes after the shape it is punched in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Icon {
    /// The polygons, in drawing order.
    pub shapes: &'static [Shape],
}

impl Icon {
    /// An icon from its shapes.
    pub const fn new(shapes: &'static [Shape]) -> Self {
        Self { shapes }
    }
}

/// One grid coordinate to a fixed-point position along an axis.
///
/// In fixed point rather than whole pixels so the filler can anti-alias the
/// edge: a triangle snapped to pixel corners at 48 px has visibly ragged
/// diagonals, and the filler is already doing the subpixel arithmetic.
#[inline]
pub fn fx_along(origin: i32, extent: i32, grid: i16) -> i32 {
    let offset = (i64::from(grid) * i64::from(extent) * i64::from(ONE)) / i64::from(GRID);
    let base = i64::from(origin.clamp(-COORD_LIMIT, COORD_LIMIT)) * i64::from(ONE);
    (base + offset).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}
