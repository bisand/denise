//! Design mode: what a press on the canvas means.
//!
//! # Why the canvas hit-tests for itself
//!
//! The tree's own hit testing is about what a *running* form should react to, and
//! it is deliberately not what a designer wants. A `Label` answers `false` to
//! `accepts_pointer`, so a click falls through it to whatever is behind — which
//! is exactly right on a panel, where a label sitting on a button must not
//! swallow the press, and exactly wrong here, where clicking a label has to
//! select the label.
//!
//! So design mode reads the events before the tree does and does its own hit
//! test, over the rectangles the reflow wrote, top-most first. Everything the
//! form contains is selectable, whether or not it would ever be pressable.
//!
//! # What an edit is
//!
//! A drag moves the node's `layout` in the tree so the person can see it, and
//! commits **once, on release**, as a targeted edit on the KDL document. One drag
//! is one document edit — which is what keeps a move to a one-line diff, and what
//! will make it one undo step in #94.

use denise::{Point, Rect};
use denise_forms::Placed;

/// How far from an edge a drag starts a resize rather than a move.
pub const HANDLE: i32 = 7;

/// How near a sibling's edge counts as lined up with it.
pub const SNAP: i32 = 4;

/// Which part of the selection a drag took hold of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    /// The body: the node moves.
    Move,
    /// A corner or an edge: the node resizes.
    Resize {
        /// Whether the left edge follows the pointer.
        left: bool,
        /// Whether the top edge follows.
        top: bool,
        /// Whether the right edge follows.
        right: bool,
        /// Whether the bottom edge follows.
        bottom: bool,
    },
}

impl Grip {
    /// The eight handles, as the overlay draws them: each is a corner or an edge.
    pub const HANDLES: [(bool, bool, bool, bool); 8] = [
        (true, true, false, false),  // north-west
        (false, true, false, false), // north
        (false, true, true, false),  // north-east
        (false, false, true, false), // east
        (false, false, true, true),  // south-east
        (false, false, false, true), // south
        (true, false, false, true),  // south-west
        (true, false, false, false), // west
    ];

    /// Where a handle sits, given the rectangle it belongs to.
    pub fn handle_rect(bounds: Rect, (left, top, right, bottom): (bool, bool, bool, bool)) -> Rect {
        let half = HANDLE / 2;
        let x = if left {
            bounds.x
        } else if right {
            bounds.x + bounds.width
        } else {
            bounds.x + bounds.width / 2
        };
        let y = if top {
            bounds.y
        } else if bottom {
            bounds.y + bounds.height
        } else {
            bounds.y + bounds.height / 2
        };
        Rect::new(x - half, y - half, HANDLE, HANDLE)
    }

    /// What a press at `at` takes hold of, for a selection at `bounds`.
    ///
    /// `None` when the press is outside the node altogether.
    pub fn at(bounds: Rect, at: Point) -> Option<Self> {
        for corner in Self::HANDLES {
            if Self::handle_rect(bounds, corner).contains(at) {
                let (left, top, right, bottom) = corner;
                return Some(Grip::Resize {
                    left,
                    top,
                    right,
                    bottom,
                });
            }
        }
        bounds.contains(at).then_some(Grip::Move)
    }
}

/// A drag in progress.
#[derive(Clone, Debug)]
pub struct Drag {
    /// What was taken hold of.
    pub grip: Grip,
    /// Where the pointer went down.
    pub from: Point,
    /// The node's rectangle, relative to its parent, when the drag began.
    pub origin: Rect,
    /// The node being dragged.
    pub path: Vec<usize>,
    /// Whether the pointer has actually moved. A press that never moves is a
    /// selection, not an edit, and must not write to the file.
    pub moved: bool,
}

/// Where a dragged edge lined up with something, for the guide to be drawn on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Guide {
    /// The position along the axis.
    pub at: i32,
    /// True for a vertical line, false for a horizontal one.
    pub vertical: bool,
}

/// The result of a drag step: where the node goes, and what it lined up with.
#[derive(Clone, Debug, Default)]
pub struct Placement {
    /// The node's new rectangle, relative to its parent.
    pub rect: Rect,
    /// Lines to draw, in the same space as `rect`.
    pub guides: Vec<Guide>,
}

/// Rounds to the nearest multiple, for a grid that is a multiple of nothing
/// interesting.
fn to_grid(value: i32, grid: i32) -> i32 {
    if grid <= 1 {
        return value;
    }
    let half = grid / 2;
    if value >= 0 {
        (value + half) / grid * grid
    } else {
        -((-value + half) / grid * grid)
    }
}

/// Rounds a whole rectangle onto the grid.
///
/// Where a widget dropped from the palette lands: the same rule a drag follows,
/// so a placed node and a moved one line up with each other.
pub fn snap(rect: Rect, grid: i32) -> Rect {
    Rect::new(
        to_grid(rect.x, grid),
        to_grid(rect.y, grid),
        to_grid(rect.width, grid).max(grid.max(1)),
        to_grid(rect.height, grid).max(grid.max(1)),
    )
}

/// The interesting positions on one axis of a rectangle: both edges and the
/// middle.
fn stops(low: i32, extent: i32) -> [i32; 3] {
    [low, low + extent / 2, low + extent]
}

/// Where a drag puts the node.
///
/// `siblings` are the rectangles of everything sharing the node's parent, in the
/// same space as `origin` — a dragged node lines up with what is beside it, which
/// is the only alignment aid that means anything without a layout engine.
pub fn place(drag: &Drag, to: Point, siblings: &[Rect], grid: i32, snapping: bool) -> Placement {
    let (dx, dy) = (to.x - drag.from.x, to.y - drag.from.y);
    let origin = drag.origin;

    let mut rect = match drag.grip {
        Grip::Move => Rect::new(origin.x + dx, origin.y + dy, origin.width, origin.height),
        Grip::Resize {
            left,
            top,
            right,
            bottom,
        } => {
            let x = if left { origin.x + dx } else { origin.x };
            let y = if top { origin.y + dy } else { origin.y };
            let width = origin.width
                + if right {
                    dx
                } else if left {
                    -dx
                } else {
                    0
                };
            let height = origin.height
                + if bottom {
                    dy
                } else if top {
                    -dy
                } else {
                    0
                };
            // A rectangle never turns itself inside out: dragging an edge past
            // its opposite stops at nothing rather than going negative, which is
            // what the tree would clamp to anyway.
            Rect::new(x, y, width.max(0), height.max(0))
        }
    };

    if !snapping {
        return Placement {
            rect,
            guides: Vec::new(),
        };
    }

    rect = Rect::new(
        to_grid(rect.x, grid),
        to_grid(rect.y, grid),
        to_grid(rect.width, grid).max(0),
        to_grid(rect.height, grid).max(0),
    );

    // Then siblings, which win over the grid: lining up with the thing beside it
    // is what somebody dragging is actually trying to do.
    let mut guides = Vec::new();
    let moving = matches!(drag.grip, Grip::Move);

    let mut best: Option<(i32, i32, i32)> = None; // distance, from, to
    for sibling in siblings {
        for &their in &stops(sibling.x, sibling.width) {
            for &mine in &stops(rect.x, rect.width) {
                let distance = (their - mine).abs();
                if distance <= SNAP && best.is_none_or(|(d, _, _)| distance < d) {
                    best = Some((distance, mine, their));
                }
            }
        }
    }
    if let Some((_, mine, their)) = best {
        let shift = their - mine;
        if moving {
            rect = Rect::new(rect.x + shift, rect.y, rect.width, rect.height);
        }
        guides.push(Guide {
            at: their,
            vertical: true,
        });
    }

    let mut best: Option<(i32, i32, i32)> = None;
    for sibling in siblings {
        for &their in &stops(sibling.y, sibling.height) {
            for &mine in &stops(rect.y, rect.height) {
                let distance = (their - mine).abs();
                if distance <= SNAP && best.is_none_or(|(d, _, _)| distance < d) {
                    best = Some((distance, mine, their));
                }
            }
        }
    }
    if let Some((_, mine, their)) = best {
        let shift = their - mine;
        if moving {
            rect = Rect::new(rect.x, rect.y + shift, rect.width, rect.height);
        }
        guides.push(Guide {
            at: their,
            vertical: false,
        });
    }

    Placement { rect, guides }
}

/// The top-most node containing `at`.
///
/// Every node counts, whether or not it would accept a pointer while the form was
/// running — see the [module docs](self) — but a node that is not *drawn* does
/// not: a form may hold an invisible sheet over its whole surface, and clicking
/// an apparently empty canvas must not select it. The outline is where a hidden
/// node is reached.
///
/// On top means later in paint order, which is `z` first and file order second —
/// the same two keys the tree sorts siblings by.
pub fn topmost(
    placed: &[Placed],
    look: impl Fn(&Placed) -> Option<(Rect, bool, i32)>,
    at: Point,
) -> Option<&Placed> {
    placed
        .iter()
        .enumerate()
        .filter_map(|(order, node)| {
            let (rect, visible, z) = look(node)?;
            (visible && rect.contains(at)).then_some((z, order, node))
        })
        .max_by_key(|&(z, order, _)| (z, order))
        .map(|(_, _, node)| node)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NODE: Rect = Rect::new(20, 30, 100, 40);

    fn drag(grip: Grip, from: Point) -> Drag {
        Drag {
            grip,
            from,
            origin: NODE,
            path: vec![0],
            moved: true,
        }
    }

    #[test]
    fn a_press_in_the_middle_moves_and_one_on_a_corner_resizes() {
        assert_eq!(Grip::at(NODE, Point::new(70, 50)), Some(Grip::Move));
        assert_eq!(
            Grip::at(NODE, Point::new(20, 30)),
            Some(Grip::Resize {
                left: true,
                top: true,
                right: false,
                bottom: false
            })
        );
        assert_eq!(
            Grip::at(NODE, Point::new(120, 70)),
            Some(Grip::Resize {
                left: false,
                top: false,
                right: true,
                bottom: true
            })
        );
        assert_eq!(Grip::at(NODE, Point::new(400, 400)), None);
    }

    #[test]
    fn the_eight_handles_are_eight_distinct_places() {
        let mut seen: Vec<Rect> = Vec::new();
        for corner in Grip::HANDLES {
            let rect = Grip::handle_rect(NODE, corner);
            assert!(!seen.contains(&rect), "two handles at {rect:?}");
            seen.push(rect);
        }
        assert_eq!(seen.len(), 8);
    }

    #[test]
    fn a_move_without_snapping_follows_the_pointer_exactly() {
        let d = drag(Grip::Move, Point::new(50, 50));
        let placed = place(&d, Point::new(57, 53), &[], 4, false);
        assert_eq!(placed.rect, Rect::new(27, 33, 100, 40));
        assert!(placed.guides.is_empty());
    }

    #[test]
    fn a_move_with_snapping_lands_on_the_grid() {
        let d = drag(Grip::Move, Point::new(50, 50));
        let placed = place(&d, Point::new(57, 53), &[], 4, true);
        assert_eq!(placed.rect.x % 4, 0, "{:?}", placed.rect);
        assert_eq!(placed.rect.y % 4, 0, "{:?}", placed.rect);
    }

    #[test]
    fn a_sibling_edge_pulls_the_node_onto_it_and_draws_a_line() {
        // A sibling whose left edge is at 26; the node lands at 27 on the grid
        // and is pulled the last pixel.
        let sibling = Rect::new(26, 200, 10, 10);
        let d = drag(Grip::Move, Point::new(50, 50));
        let placed = place(&d, Point::new(58, 50), &[sibling], 1, true);
        assert_eq!(placed.rect.x, 26, "{:?}", placed.rect);
        assert!(
            placed.guides.contains(&Guide {
                at: 26,
                vertical: true
            }),
            "{:?}",
            placed.guides
        );
    }

    #[test]
    fn a_resize_moves_the_edge_that_was_taken_hold_of_and_no_other() {
        let d = drag(
            Grip::Resize {
                left: false,
                top: false,
                right: true,
                bottom: false,
            },
            Point::new(120, 50),
        );
        let placed = place(&d, Point::new(140, 90), &[], 1, false);
        assert_eq!(placed.rect.x, NODE.x, "the left edge moved");
        assert_eq!(placed.rect.y, NODE.y);
        assert_eq!(placed.rect.height, NODE.height, "the height changed");
        assert_eq!(placed.rect.width, NODE.width + 20);
    }

    #[test]
    fn dragging_an_edge_past_its_opposite_stops_at_nothing() {
        let d = drag(
            Grip::Resize {
                left: false,
                top: false,
                right: true,
                bottom: true,
            },
            Point::new(120, 70),
        );
        let placed = place(&d, Point::new(-500, -500), &[], 1, false);
        assert_eq!(placed.rect.width, 0);
        assert_eq!(placed.rect.height, 0);
    }

    #[test]
    fn the_grid_rounds_both_ways_from_zero() {
        assert_eq!(to_grid(0, 4), 0);
        assert_eq!(to_grid(1, 4), 0);
        assert_eq!(to_grid(2, 4), 4);
        assert_eq!(to_grid(-2, 4), -4);
        assert_eq!(to_grid(-1, 4), 0);
        assert_eq!(to_grid(7, 1), 7, "a grid of one is no grid");
    }
}
