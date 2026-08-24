//! What the arrange commands do to a set of rectangles.
//!
//! # Why this is siblings only
//!
//! A rectangle in a form file is relative to its parent and there is no layout
//! engine, so "line these up" is only a question that has an answer when they
//! are all measured from the same corner. Two nodes in different panels have no
//! shared space to be aligned in: the numbers would have to be translated
//! through both parents, and the answer would stop being true the moment either
//! panel moved. The commands are disabled rather than doing something that looks
//! right once.
//!
//! # The anchor
//!
//! Everything here follows **one** of the selection and leaves it exactly where
//! it is — Delphi's rule, and the only one that produces a stable answer when
//! the command is given twice.
//!
//! [The issue](https://github.com/bisand/denise/issues/95) asked for the *first*
//! selected node. The designer uses the **primary** one instead — the last one
//! taken, the one wearing the handles — because a rubber band has no
//! first-clicked node to point at: it takes what it encloses in file order, and
//! neither end of that list means anything to the person who drew it. The one
//! wearing the handles is the one that can be seen.

use denise::Rect;

/// One of the arrange commands, in the order the bar shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Left edges to the anchor's.
    Left,
    /// Horizontal centres to the anchor's.
    CentreAcross,
    /// Right edges to the anchor's.
    Right,
    /// Top edges to the anchor's.
    Top,
    /// Vertical centres to the anchor's.
    CentreDown,
    /// Bottom edges to the anchor's.
    Bottom,
    /// The anchor's width.
    SameWidth,
    /// The anchor's height.
    SameHeight,
    /// Both.
    SameSize,
    /// Equal gaps left to right; the outermost two stay.
    SpaceAcross,
    /// Equal gaps top to bottom; the outermost two stay.
    SpaceDown,
    /// Into a new panel that takes their bounding box.
    Group,
    /// Out of the panel that holds them.
    Ungroup,
}

/// What a command needs before it means anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Needs {
    /// Two or more nodes, all with the same parent.
    Several,
    /// Three or more: with two there is no gap between to make equal.
    Spread,
    /// One node, and one that holds children.
    Holder,
}

impl Needs {
    /// Why the command is greyed out, for the tooltip that says so.
    pub const fn why(self) -> &'static str {
        match self {
            Self::Several => "Select two or more nodes with the same parent",
            Self::Spread => "Select three or more nodes with the same parent",
            Self::Holder => "Select one panel, with something in it",
        }
    }
}

impl Command {
    /// Every command, in the order the bar lays them out.
    pub const ALL: [Self; 13] = [
        Self::Left,
        Self::CentreAcross,
        Self::Right,
        Self::Top,
        Self::CentreDown,
        Self::Bottom,
        Self::SameWidth,
        Self::SameHeight,
        Self::SameSize,
        Self::SpaceAcross,
        Self::SpaceDown,
        Self::Group,
        Self::Ungroup,
    ];

    /// What the button says.
    ///
    /// Two characters at most, because there are thirteen of them and the font
    /// this toolkit ships is ASCII and Latin-1 — the arrows and boxes an icon
    /// would want are not in it, and would draw as empty squares. The tooltip is
    /// where the words are.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Left => "L",
            Self::CentreAcross => "C",
            Self::Right => "R",
            Self::Top => "T",
            Self::CentreDown => "M",
            Self::Bottom => "B",
            Self::SameWidth => "W",
            Self::SameHeight => "H",
            Self::SameSize => "WH",
            Self::SpaceAcross => "-",
            Self::SpaceDown => "|",
            Self::Group => "[+]",
            Self::Ungroup => "[-]",
        }
    }

    /// What the button's tooltip says when it can be pressed.
    pub const fn what(self) -> &'static str {
        match self {
            Self::Left => "Align left edges to the one with the handles",
            Self::CentreAcross => "Align horizontal centres to the one with the handles",
            Self::Right => "Align right edges to the one with the handles",
            Self::Top => "Align top edges to the one with the handles",
            Self::CentreDown => "Align vertical centres to the one with the handles",
            Self::Bottom => "Align bottom edges to the one with the handles",
            Self::SameWidth => "Same width as the one with the handles",
            Self::SameHeight => "Same height as the one with the handles",
            Self::SameSize => "Same width and height as the one with the handles",
            Self::SpaceAcross => "Equal gaps left to right; the outermost two stay",
            Self::SpaceDown => "Equal gaps top to bottom; the outermost two stay",
            Self::Group => "Into a new panel that takes their bounding box",
            Self::Ungroup => "Out of the panel that holds them, and take it away",
        }
    }

    /// What has to be selected for it.
    pub const fn needs(self) -> Needs {
        match self {
            Self::SpaceAcross | Self::SpaceDown => Needs::Spread,
            Self::Ungroup => Needs::Holder,
            _ => Needs::Several,
        }
    }

    /// Whether this one is a move in the tree rather than a change of numbers.
    ///
    /// [`arrange`] answers the rest; these two are the document's shape and
    /// belong where the document is.
    pub const fn is_structural(self) -> bool {
        matches!(self, Self::Group | Self::Ungroup)
    }

    /// A short name for the status line.
    pub const fn done(self) -> &'static str {
        match self {
            Self::Left => "aligned left",
            Self::CentreAcross => "aligned on the centre",
            Self::Right => "aligned right",
            Self::Top => "aligned top",
            Self::CentreDown => "aligned on the middle",
            Self::Bottom => "aligned bottom",
            Self::SameWidth => "made the same width",
            Self::SameHeight => "made the same height",
            Self::SameSize => "made the same size",
            Self::SpaceAcross => "spaced evenly across",
            Self::SpaceDown => "spaced evenly down",
            Self::Group => "grouped",
            Self::Ungroup => "ungrouped",
        }
    }
}

/// Where a command puts each rectangle.
///
/// `rects` are siblings, in the same space; `anchor` indexes the one that stays
/// put. The answer is in the same order, so the caller can pair it back up with
/// the paths it came from and write only what changed.
///
/// A structural command ([`Command::is_structural`]) moves nodes in the tree
/// rather than changing their numbers, and comes back unchanged.
pub fn arrange(command: Command, rects: &[Rect], anchor: usize) -> Vec<Rect> {
    let mut out = rects.to_vec();
    let Some(&fixed) = rects.get(anchor) else {
        return out;
    };

    match command {
        Command::Left => set(&mut out, |r| Rect::new(fixed.x, r.y, r.width, r.height)),
        Command::CentreAcross => set(&mut out, |r| {
            Rect::new(
                fixed.x + (fixed.width - r.width) / 2,
                r.y,
                r.width,
                r.height,
            )
        }),
        Command::Right => set(&mut out, |r| {
            Rect::new(fixed.right() - r.width, r.y, r.width, r.height)
        }),
        Command::Top => set(&mut out, |r| Rect::new(r.x, fixed.y, r.width, r.height)),
        Command::CentreDown => set(&mut out, |r| {
            Rect::new(
                r.x,
                fixed.y + (fixed.height - r.height) / 2,
                r.width,
                r.height,
            )
        }),
        Command::Bottom => set(&mut out, |r| {
            Rect::new(r.x, fixed.bottom() - r.height, r.width, r.height)
        }),
        Command::SameWidth => set(&mut out, |r| Rect::new(r.x, r.y, fixed.width, r.height)),
        Command::SameHeight => set(&mut out, |r| Rect::new(r.x, r.y, r.width, fixed.height)),
        Command::SameSize => set(&mut out, |r| Rect::new(r.x, r.y, fixed.width, fixed.height)),
        Command::SpaceAcross => spread(&mut out, false),
        Command::SpaceDown => spread(&mut out, true),
        Command::Group | Command::Ungroup => {}
    }
    out
}

/// Applies a rule to every rectangle. The anchor is included and comes out
/// unchanged by every rule above, which is what makes giving a command twice the
/// same as giving it once.
fn set(rects: &mut [Rect], rule: impl Fn(Rect) -> Rect) {
    for rect in rects.iter_mut() {
        *rect = rule(*rect);
    }
}

/// Equal gaps along one axis, with the outermost two left where they are.
///
/// Gaps and not centres: with widgets of different widths, equal gaps is what
/// the eye reads as evenly spaced, and equal centres is what a table wants. This
/// is the one the eye reads.
fn spread(rects: &mut [Rect], down: bool) {
    if rects.len() < 3 {
        return;
    }
    let low = |r: &Rect| if down { r.y } else { r.x };
    let extent = |r: &Rect| if down { r.height } else { r.width };

    let mut order: Vec<usize> = (0..rects.len()).collect();
    // By position, and by their original order where two start in the same
    // place — so the answer does not depend on how the sort happened to break a
    // tie.
    order.sort_by_key(|&index| (low(&rects[index]), index));

    let first = rects[order[0]];
    let last = rects[order[order.len() - 1]];
    let span = low(&last) + extent(&last) - low(&first);
    let occupied: i32 = order.iter().map(|&index| extent(&rects[index])).sum();
    let gap = (span - occupied) / (order.len() as i32 - 1);

    let mut cursor = low(&first) + extent(&first) + gap;
    for &index in &order[1..order.len() - 1] {
        let rect = &mut rects[index];
        *rect = if down {
            Rect::new(rect.x, cursor, rect.width, rect.height)
        } else {
            Rect::new(cursor, rect.y, rect.width, rect.height)
        };
        cursor += extent(rect) + gap;
    }
}

/// The smallest rectangle holding all of them.
pub fn bounds(rects: &[Rect]) -> Rect {
    rects
        .iter()
        .copied()
        .reduce(|all, one| all.union(&one))
        .unwrap_or(Rect::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three of different sizes, deliberately not in position order.
    fn three() -> Vec<Rect> {
        vec![
            Rect::new(10, 10, 40, 20),
            Rect::new(100, 50, 60, 40),
            Rect::new(60, 90, 20, 10),
        ]
    }

    #[test]
    fn aligning_moves_everything_to_the_anchor_and_leaves_the_anchor_alone() {
        let rects = three();
        let out = arrange(Command::Left, &rects, 1);
        assert_eq!(out[1], rects[1], "the anchor moved");
        assert!(out.iter().all(|r| r.x == 100), "{out:?}");
        // And nothing about the other axis was touched.
        assert_eq!(out[0].y, rects[0].y);
        assert_eq!(out[2].height, rects[2].height);
    }

    #[test]
    fn every_alignment_puts_the_edge_it_names_where_the_anchors_is() {
        let rects = three();
        let anchor = rects[0];
        for (command, of) in [
            (Command::Left, (anchor.x, true)),
            (Command::Right, (anchor.right(), true)),
            (Command::Top, (anchor.y, false)),
            (Command::Bottom, (anchor.bottom(), false)),
        ] {
            let out = arrange(command, &rects, 0);
            for rect in &out {
                let (edge, across) = of;
                let mine = match (command, across) {
                    (Command::Left, _) => rect.x,
                    (Command::Right, _) => rect.right(),
                    (Command::Top, _) => rect.y,
                    _ => rect.bottom(),
                };
                assert_eq!(mine, edge, "{command:?} left {rect:?} behind");
            }
        }
    }

    #[test]
    fn centring_puts_the_middles_together() {
        let rects = three();
        let out = arrange(Command::CentreAcross, &rects, 2);
        let middle = |r: &Rect| r.x + r.width / 2;
        assert!(
            out.iter().all(|r| middle(r) == middle(&rects[2])),
            "{out:?}"
        );
        let out = arrange(Command::CentreDown, &rects, 2);
        let middle = |r: &Rect| r.y + r.height / 2;
        assert!(
            out.iter().all(|r| middle(r) == middle(&rects[2])),
            "{out:?}"
        );
    }

    #[test]
    fn the_same_size_is_the_anchors_size_and_nobody_moves() {
        let rects = three();
        let out = arrange(Command::SameSize, &rects, 1);
        for (before, after) in rects.iter().zip(&out) {
            assert_eq!((after.x, after.y), (before.x, before.y), "it moved");
            assert_eq!((after.width, after.height), (60, 40));
        }
        let out = arrange(Command::SameWidth, &rects, 1);
        assert!(out.iter().all(|r| r.width == 60));
        assert_eq!(out[0].height, rects[0].height, "the height changed too");

        let out = arrange(Command::SameHeight, &rects, 1);
        assert!(out.iter().all(|r| r.height == 40));
        assert_eq!(out[0].width, rects[0].width, "the width changed too");
    }

    #[test]
    fn giving_a_command_twice_is_the_same_as_giving_it_once() {
        let rects = three();
        for command in Command::ALL {
            let once = arrange(command, &rects, 1);
            let twice = arrange(command, &once, 1);
            assert_eq!(once, twice, "{command:?} is not settled after one go");
        }
    }

    #[test]
    fn spreading_leaves_the_outermost_two_and_makes_the_gaps_equal() {
        // Left edges at 0, 40, 200; widths 10, 20, 10.
        let rects = vec![
            Rect::new(0, 0, 10, 10),
            Rect::new(40, 0, 20, 10),
            Rect::new(200, 0, 10, 10),
        ];
        let out = arrange(Command::SpaceAcross, &rects, 0);
        assert_eq!(out[0], rects[0], "the leftmost moved");
        assert_eq!(out[2], rects[2], "the rightmost moved");
        // 210 across, 40 of it occupied, so 170 to share over two gaps.
        assert_eq!(out[1].x, 10 + 85);
        assert_eq!(out[2].x - (out[1].x + out[1].width), 85);
    }

    #[test]
    fn spreading_sorts_by_position_rather_than_by_the_order_they_were_picked() {
        // The middle one is *second* by position and last in the list.
        let rects = vec![
            Rect::new(0, 0, 10, 10),
            Rect::new(200, 0, 10, 10),
            Rect::new(40, 0, 20, 10),
        ];
        let out = arrange(Command::SpaceAcross, &rects, 0);
        assert_eq!(out[0], rects[0]);
        assert_eq!(
            out[1], rects[1],
            "the rightmost was treated as a middle one"
        );
        assert_eq!(out[2].x, 10 + 85);
    }

    #[test]
    fn spreading_two_has_nothing_between_them_to_space() {
        let rects = vec![Rect::new(0, 0, 10, 10), Rect::new(200, 0, 10, 10)];
        assert_eq!(arrange(Command::SpaceAcross, &rects, 0), rects);
    }

    #[test]
    fn spreading_down_is_the_same_rule_on_the_other_axis() {
        let rects = vec![
            Rect::new(0, 0, 10, 10),
            Rect::new(0, 40, 10, 20),
            Rect::new(0, 200, 10, 10),
        ];
        let out = arrange(Command::SpaceDown, &rects, 0);
        assert_eq!(out[1].y, 10 + 85);
        assert_eq!(out[1].x, rects[1].x, "it moved sideways");
    }

    #[test]
    fn a_structural_command_changes_no_numbers_here() {
        let rects = three();
        assert_eq!(arrange(Command::Group, &rects, 0), rects);
        assert_eq!(arrange(Command::Ungroup, &rects, 0), rects);
        assert!(Command::Group.is_structural());
        assert!(!Command::Left.is_structural());
    }

    #[test]
    fn the_bounding_box_holds_all_of_them() {
        let all = bounds(&three());
        assert_eq!(all, Rect::new(10, 10, 150, 90));
        assert_eq!(bounds(&[]), Rect::ZERO);
    }

    #[test]
    fn every_command_has_a_label_short_enough_to_fit_and_words_to_explain_it() {
        for command in Command::ALL {
            assert!(
                (1..=3).contains(&command.label().len()),
                "{command:?}: {}",
                command.label()
            );
            assert!(command.what().len() > 10, "{command:?} explains nothing");
            assert!(command.done().len() > 3, "{command:?} says nothing after");
            assert!(command.needs().why().len() > 10, "{command:?}");
        }
    }
}
