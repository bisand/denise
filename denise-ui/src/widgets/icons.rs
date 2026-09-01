//! Every widget's glyph: a portrait of the thing, drawn rather than looked up.
//!
//! A palette that lists twenty-six widgets by name serves the person who
//! already knows which one they want. A glyph beside the name serves everybody
//! else — and the glyph has to be drawable on every machine, which rules out an
//! icon font before the argument starts: [`denise_render::icon`] exists because
//! whether a font has a picture depends on which font is installed, and these
//! are the same pictures with the same problem.
//!
//! # The recipe
//!
//! Every glyph here follows one rule set, so the set reads as a set:
//!
//! - **Solid, outlined by knockout.** The format has no stroke — see
//!   [`denise_render::icon`] for why — and at the sixteen pixels a palette row
//!   offers, a thin outline would vanish anyway. A shape that reads as an
//!   outline is a fill with the middle knocked back out in [`Ink::Back`],
//!   exactly as the keyboard's Backspace draws its cross.
//! - **Line weights of 7–12 grid units**, so nothing drops below a pixel at
//!   sixteen pixels square.
//! - **Circles are 8-, 10-, 12- and 16-gons**, as the keyboard's globe is a
//!   22-gon: there is no curve primitive, and at this size there is no
//!   difference.
//! - **A portrait of the widget's anatomy**, not a metaphor for it: the toggle
//!   is a pill with its knob, the select is a field with its arrow, the tree is
//!   a stem with its elbows. A person who has seen the widget recognises the
//!   glyph.
//!
//! Not miniature live widgets, deliberately: a real `Table` rasterised into
//! sixteen pixels is mud, themed fills would make a palette shimmer, and every
//! stateful widget would need a state picked for it. Stylised beats shrunken.
//!
//! # Where they are wired up
//!
//! A widget names its glyph through [`Describe::ICON`](super::Describe::ICON),
//! next to its `DOC` and its `GROUP`, and [`all`](super::all) carries it in
//! [`WidgetInfo`](super::WidgetInfo) — so a palette draws glyphs without naming
//! widgets, and the twenty-seventh widget cannot be merged without one. The
//! tests at the bottom hold every glyph to the format's real limits.
//!
//! [`Ink::Back`]: denise_render::icon::Ink::Back

use denise_render::icon::{Icon, Shape};

// ------------------------------------------------------------------- input

/// Button: a rounded slab with its label knocked out.
pub static BUTTON: Icon = Icon::new(&[
    Shape::fore(&[
        (22, 26),
        (78, 26),
        (88, 36),
        (88, 64),
        (78, 74),
        (22, 74),
        (12, 64),
        (12, 36),
    ]),
    Shape::back(&[(30, 45), (70, 45), (70, 55), (30, 55)]),
]);

/// Checkbox: a rounded box with the tick knocked out — the state worth drawing.
pub static CHECKBOX: Icon = Icon::new(&[
    Shape::fore(&[
        (24, 14),
        (76, 14),
        (86, 24),
        (86, 76),
        (76, 86),
        (24, 86),
        (14, 76),
        (14, 24),
    ]),
    // The tick is one polygon: up the left arm, over the valley, up the long
    // arm to the tip, and back along the underside.
    Shape::back(&[(18, 54), (30, 42), (42, 54), (68, 26), (80, 38), (42, 76)]),
]);

/// Radio group: a ring with the chosen disc in it.
pub static RADIO_GROUP: Icon = Icon::new(&[
    Shape::fore(&[
        (50, 12),
        (65, 15),
        (77, 23),
        (85, 35),
        (88, 50),
        (85, 65),
        (77, 77),
        (65, 85),
        (50, 88),
        (35, 85),
        (23, 77),
        (15, 65),
        (12, 50),
        (15, 35),
        (23, 23),
        (35, 15),
    ]),
    Shape::back(&[
        (50, 24),
        (60, 26),
        (68, 32),
        (74, 40),
        (76, 50),
        (74, 60),
        (68, 68),
        (60, 74),
        (50, 76),
        (40, 74),
        (32, 68),
        (26, 60),
        (24, 50),
        (26, 40),
        (32, 32),
        (40, 26),
    ]),
    Shape::fore(&[
        (50, 36),
        (57, 38),
        (62, 43),
        (64, 50),
        (62, 57),
        (57, 62),
        (50, 64),
        (43, 62),
        (38, 57),
        (36, 50),
        (38, 43),
        (43, 38),
    ]),
]);

/// Toggle: the pill with the knob knocked out — a switch, on.
pub static TOGGLE: Icon = Icon::new(&[
    Shape::fore(&[
        (30, 28),
        (70, 28),
        (84, 36),
        (90, 50),
        (84, 64),
        (70, 72),
        (30, 72),
        (16, 64),
        (10, 50),
        (16, 36),
    ]),
    Shape::back(&[
        (66, 35),
        (74, 37),
        (79, 42),
        (81, 50),
        (79, 58),
        (74, 63),
        (66, 65),
        (58, 63),
        (53, 58),
        (51, 50),
        (53, 42),
        (58, 37),
    ]),
]);

/// Slider: a track with its knob partway along.
pub static SLIDER: Icon = Icon::new(&[
    Shape::fore(&[(8, 46), (92, 46), (92, 54), (8, 54)]),
    Shape::fore(&[
        (62, 35),
        (70, 37),
        (75, 42),
        (77, 50),
        (75, 58),
        (70, 63),
        (62, 65),
        (54, 63),
        (49, 58),
        (47, 50),
        (49, 42),
        (54, 37),
    ]),
]);

/// Text input: an outlined field with the caret standing in it.
pub static TEXT_INPUT: Icon = Icon::new(&[
    Shape::fore(&[(8, 28), (92, 28), (92, 72), (8, 72)]),
    Shape::back(&[(15, 35), (85, 35), (85, 65), (15, 65)]),
    Shape::fore(&[(24, 42), (30, 42), (30, 58), (24, 58)]),
]);

/// Select: an outlined field with the arrow that opens it.
pub static SELECT: Icon = Icon::new(&[
    Shape::fore(&[(8, 28), (92, 28), (92, 72), (8, 72)]),
    Shape::back(&[(15, 35), (85, 35), (85, 65), (15, 65)]),
    Shape::fore(&[(58, 44), (78, 44), (68, 58)]),
]);

/// Rating: one star, which is what the widget draws five of.
pub static RATING: Icon = Icon::new(&[Shape::fore(&[
    (50, 11),
    (60, 39),
    (90, 40),
    (66, 58),
    (75, 87),
    (50, 70),
    (25, 87),
    (34, 58),
    (10, 40),
    (40, 39),
])]);

// ----------------------------------------------------------------- display

/// Label: two lines of text, the second shorter, as prose is.
pub static LABEL: Icon = Icon::new(&[
    Shape::fore(&[(10, 30), (90, 30), (90, 42), (10, 42)]),
    Shape::fore(&[(10, 58), (62, 58), (62, 70), (10, 70)]),
]);

/// Badge: the pill itself, solid — a badge is a filled thing.
pub static BADGE: Icon = Icon::new(&[Shape::fore(&[
    (32, 34),
    (68, 34),
    (80, 42),
    (84, 50),
    (80, 58),
    (68, 66),
    (32, 66),
    (20, 58),
    (16, 50),
    (20, 42),
])]);

/// Alert: the warning triangle with the mark knocked out of it.
pub static ALERT: Icon = Icon::new(&[
    Shape::fore(&[(50, 10), (93, 84), (7, 84)]),
    Shape::back(&[(46, 34), (54, 34), (54, 58), (46, 58)]),
    Shape::back(&[(46, 66), (54, 66), (54, 74), (46, 74)]),
]);

/// Divider: the rule, running wider than the content it separates.
pub static DIVIDER: Icon = Icon::new(&[
    Shape::fore(&[(20, 16), (80, 16), (80, 28), (20, 28)]),
    Shape::fore(&[(6, 46), (94, 46), (94, 54), (6, 54)]),
    Shape::fore(&[(20, 72), (80, 72), (80, 84), (20, 84)]),
]);

// --------------------------------------------------------------- indicator

/// Progress: an outlined track, filled partway.
pub static PROGRESS: Icon = Icon::new(&[
    Shape::fore(&[(8, 36), (92, 36), (92, 64), (8, 64)]),
    Shape::back(&[(15, 43), (85, 43), (85, 57), (15, 57)]),
    Shape::fore(&[(15, 43), (56, 43), (56, 57), (15, 57)]),
]);

/// Radial progress: the ring with a quarter still to go.
///
/// The ring is the disc with the middle knocked out; the gap is a quarter
/// knocked out after it, opening to the upper right where the widget's own
/// sweep would be heading.
pub static RADIAL_PROGRESS: Icon = Icon::new(&[
    Shape::fore(&[
        (50, 10),
        (65, 13),
        (78, 22),
        (87, 35),
        (90, 50),
        (87, 65),
        (78, 78),
        (65, 87),
        (50, 90),
        (35, 87),
        (22, 78),
        (13, 65),
        (10, 50),
        (13, 35),
        (22, 22),
        (35, 13),
    ]),
    Shape::back(&[
        (50, 24),
        (60, 26),
        (68, 32),
        (74, 40),
        (76, 50),
        (74, 60),
        (68, 68),
        (60, 74),
        (50, 76),
        (40, 74),
        (32, 68),
        (26, 60),
        (24, 50),
        (26, 40),
        (32, 32),
        (40, 26),
    ]),
    Shape::back(&[(50, 50), (50, 2), (98, 2), (98, 50)]),
]);

/// Spinner: three discs on the turn, which is the widget's whole story.
pub static SPINNER: Icon = Icon::new(&[
    Shape::fore(&[
        (50, 6),
        (58, 9),
        (61, 17),
        (58, 25),
        (50, 28),
        (42, 25),
        (39, 17),
        (42, 9),
    ]),
    Shape::fore(&[
        (79, 56),
        (87, 59),
        (90, 67),
        (87, 75),
        (79, 78),
        (71, 75),
        (68, 67),
        (71, 59),
    ]),
    Shape::fore(&[
        (21, 56),
        (29, 59),
        (32, 67),
        (29, 75),
        (21, 78),
        (13, 75),
        (10, 67),
        (13, 59),
    ]),
]);

// --------------------------------------------------------------- container

/// Panel: an outlined box with nothing in it, which is what a panel is for.
pub static PANEL: Icon = Icon::new(&[
    Shape::fore(&[
        (18, 16),
        (82, 16),
        (90, 24),
        (90, 76),
        (82, 84),
        (18, 84),
        (10, 76),
        (10, 24),
    ]),
    Shape::back(&[
        (24, 24),
        (76, 24),
        (82, 30),
        (82, 70),
        (76, 76),
        (24, 76),
        (18, 70),
        (18, 30),
    ]),
]);

/// Tabs: the raised tab joined to its baseline, and the one behind it.
pub static TABS: Icon = Icon::new(&[
    Shape::fore(&[(8, 66), (8, 34), (42, 34), (42, 58), (92, 58), (92, 66)]),
    Shape::fore(&[(50, 42), (84, 42), (84, 58), (50, 58)]),
]);

/// Collapse: a header with its chevron, over the content it folds away.
pub static COLLAPSE: Icon = Icon::new(&[
    Shape::fore(&[(8, 20), (92, 20), (92, 46), (8, 46)]),
    Shape::back(&[(38, 27), (50, 37), (62, 27), (62, 35), (50, 45), (38, 35)]),
    Shape::fore(&[(8, 58), (92, 58), (92, 66), (8, 66)]),
    Shape::fore(&[(8, 74), (64, 74), (64, 82), (8, 82)]),
]);

// -------------------------------------------------------------------- data

/// List: three rows, each a bullet and its label. Exactly [`MAX_SHAPES`].
///
/// [`MAX_SHAPES`]: denise_render::icon::MAX_SHAPES
pub static LIST: Icon = Icon::new(&[
    Shape::fore(&[(12, 18), (22, 18), (22, 28), (12, 28)]),
    Shape::fore(&[(30, 18), (88, 18), (88, 28), (30, 28)]),
    Shape::fore(&[(12, 45), (22, 45), (22, 55), (12, 55)]),
    Shape::fore(&[(30, 45), (88, 45), (88, 55), (30, 55)]),
    Shape::fore(&[(12, 72), (22, 72), (22, 82), (12, 82)]),
    Shape::fore(&[(30, 72), (88, 72), (88, 82), (30, 82)]),
]);

/// Table: a solid header band over four knocked-out cells.
pub static TABLE: Icon = Icon::new(&[
    Shape::fore(&[(8, 20), (92, 20), (92, 80), (8, 80)]),
    Shape::back(&[(16, 34), (46, 34), (46, 52), (16, 52)]),
    Shape::back(&[(54, 34), (84, 34), (84, 52), (54, 52)]),
    Shape::back(&[(16, 60), (46, 60), (46, 72), (16, 72)]),
    Shape::back(&[(54, 60), (84, 60), (84, 72), (54, 72)]),
]);

/// Tree: the root, and a stem turning two elbows out to its children.
pub static TREE: Icon = Icon::new(&[
    Shape::fore(&[(10, 12), (22, 12), (22, 24), (10, 24)]),
    Shape::fore(&[(30, 13), (84, 13), (84, 23), (30, 23)]),
    // The stem and both elbows are one polygon: down the outside, in and back
    // along each elbow on the way.
    Shape::fore(&[
        (14, 24),
        (18, 24),
        (18, 44),
        (34, 44),
        (34, 52),
        (18, 52),
        (18, 73),
        (34, 73),
        (34, 81),
        (14, 81),
    ]),
    Shape::fore(&[(40, 43), (84, 43), (84, 53), (40, 53)]),
    Shape::fore(&[(40, 72), (84, 72), (84, 82), (40, 82)]),
]);

/// Timeline: the line, two events on it, and their labels.
pub static TIMELINE: Icon = Icon::new(&[
    Shape::fore(&[(26, 8), (32, 8), (32, 92), (26, 92)]),
    Shape::fore(&[
        (29, 17),
        (37, 20),
        (40, 28),
        (37, 36),
        (29, 39),
        (21, 36),
        (18, 28),
        (21, 20),
    ]),
    Shape::fore(&[
        (29, 59),
        (37, 62),
        (40, 70),
        (37, 78),
        (29, 81),
        (21, 78),
        (18, 70),
        (21, 62),
    ]),
    Shape::fore(&[(44, 22), (90, 22), (90, 34), (44, 34)]),
    Shape::fore(&[(44, 64), (90, 64), (90, 76), (44, 76)]),
]);

// ------------------------------------------------------------------- media

/// Image: the frame, the mountains, the sun — the picture every OS draws for
/// a picture.
pub static IMAGE: Icon = Icon::new(&[
    Shape::fore(&[(8, 18), (92, 18), (92, 82), (8, 82)]),
    Shape::back(&[(15, 25), (85, 25), (85, 75), (15, 75)]),
    Shape::fore(&[(18, 75), (40, 44), (54, 61), (64, 50), (85, 75)]),
    Shape::fore(&[
        (68, 30),
        (73, 32),
        (75, 37),
        (73, 42),
        (68, 44),
        (63, 42),
        (61, 37),
        (63, 32),
    ]),
]);

/// Video: the frame with the play mark knocked out of it.
pub static VIDEO: Icon = Icon::new(&[
    Shape::fore(&[
        (16, 22),
        (84, 22),
        (92, 30),
        (92, 70),
        (84, 78),
        (16, 78),
        (8, 70),
        (8, 30),
    ]),
    Shape::back(&[(40, 35), (68, 50), (40, 65)]),
]);

/// Avatar: the disc with a head and shoulders knocked out.
///
/// The shoulders close below the disc's bottom edge on purpose: back ink
/// outside the disc paints the background its own colour, which is nothing,
/// and closing higher would leave a sliver of disc under them.
pub static AVATAR: Icon = Icon::new(&[
    Shape::fore(&[
        (50, 10),
        (65, 13),
        (78, 22),
        (87, 35),
        (90, 50),
        (87, 65),
        (78, 78),
        (65, 87),
        (50, 90),
        (35, 87),
        (22, 78),
        (13, 65),
        (10, 50),
        (13, 35),
        (22, 22),
        (35, 13),
    ]),
    Shape::back(&[
        (50, 26),
        (58, 28),
        (62, 35),
        (62, 43),
        (58, 50),
        (50, 52),
        (42, 50),
        (38, 43),
        (38, 35),
        (42, 28),
    ]),
    Shape::back(&[(26, 92), (30, 64), (42, 56), (58, 56), (70, 64), (74, 92)]),
]);

/// Carousel: the slide in front, and the edges of its neighbours.
pub static CAROUSEL: Icon = Icon::new(&[
    Shape::fore(&[(24, 24), (76, 24), (76, 76), (24, 76)]),
    Shape::back(&[(31, 31), (69, 31), (69, 69), (31, 69)]),
    Shape::fore(&[(8, 34), (16, 34), (16, 66), (8, 66)]),
    Shape::fore(&[(84, 34), (92, 34), (92, 66), (84, 66)]),
]);

#[cfg(test)]
mod tests {
    use denise_render::MAX_ICON_VERTICES;
    use denise_render::icon::{GRID, MAX_SHAPES};

    use crate::widgets::all;

    /// Every glyph fits the format it is written in. The rasteriser ignores
    /// what is over these limits rather than drawing it wrong, so a glyph that
    /// broke them would ship missing its later shapes and nobody would see why.
    #[test]
    fn every_glyph_fits_the_format() {
        for widget in all() {
            let icon = widget.icon;
            assert!(
                (1..=MAX_SHAPES).contains(&icon.shapes.len()),
                "{}: {} shapes",
                widget.kind,
                icon.shapes.len()
            );
            for shape in icon.shapes {
                assert!(
                    (3..=MAX_ICON_VERTICES).contains(&shape.points.len()),
                    "{}: a shape with {} vertices",
                    widget.kind,
                    shape.points.len()
                );
                for (x, y) in shape.points {
                    assert!(
                        (0..=GRID as i16).contains(x) && (0..=GRID as i16).contains(y),
                        "{}: ({x}, {y}) is off the grid",
                        widget.kind
                    );
                }
            }
        }
    }

    /// No two widgets wear the same picture — the point of a glyph is telling
    /// a `checkbox` from a `toggle` at a glance, same as `DOC`'s rule.
    #[test]
    fn no_two_widgets_share_a_glyph() {
        let widgets = all();
        for (i, a) in widgets.iter().enumerate() {
            for b in &widgets[i + 1..] {
                assert_ne!(
                    a.icon, b.icon,
                    "{} and {} wear the same glyph",
                    a.kind, b.kind
                );
            }
        }
    }

    /// A knockout with nothing under it is a shape the background paints over
    /// itself: drawing order starts with ink somebody will see.
    #[test]
    fn every_glyph_starts_with_fore_ink() {
        use denise_render::icon::Ink;
        for widget in all() {
            let first = widget.icon.shapes.first().expect("at least one shape");
            assert_eq!(
                first.ink,
                Ink::Fore,
                "{} opens with a knockout",
                widget.kind
            );
        }
    }
}
