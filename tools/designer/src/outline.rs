//! The lower-left pane: the form's tree, drawn as a tree.
//!
//! The canvas cannot show everything a form has. A node behind another, clipped
//! out of its parent, sized to nothing, or one of a hundred identical rows — all
//! of those are on the canvas and none of them can be pointed at. So the outline
//! lists every node, indented, and is the way to reach them.
//!
//! # Why this is not a `List`
//!
//! A row has **three things to press**: the triangle that folds the subtree, the
//! row itself that selects, and an eye that hides the node so whatever is under
//! it can be reached. A `List` row is one hit target.
//!
//! So the pane is drawn out of panels and labels and hit-tested here, which is
//! what the property inspector already does and what design mode already does to
//! the canvas and the palette. A `Tree` widget ([#128]) would not change that
//! unless it grew per-row controls; what it will add later is the keyboard walk.
//!
//! [#128]: https://github.com/bisand/denise/issues/128

use denise::{Point, Rect, Role};
use denise_ui::widgets::{Label, Panel};
use denise_ui::{NodeId, TextStyle, Ui};

use crate::app::Message;

/// A row's height, and how far one step of nesting moves it across.
pub const ROW: i32 = 22;
const INDENT: i32 = 12;
/// The triangle, and the eye.
const FOLD: i32 = 13;
const EYE: i32 = 15;
const GAP: i32 = 3;

/// One row of the outline: a node, and what is known about it here.
#[derive(Clone, Debug)]
pub struct Row {
    /// The node, by the path that survives a rebuild.
    pub path: Vec<usize>,
    /// What kind of widget it is.
    pub kind: &'static str,
    /// The name the file gave it, if it gave one.
    pub name: Option<String>,
    /// How deep it sits.
    pub depth: usize,
    /// Whether it has children in the file.
    pub parent: bool,
    /// Whether they are being shown.
    pub open: bool,
    /// Whether the *file* hides it, with `visible=#false`.
    pub by_file: bool,
    /// Whether the **designer** hides it, which the file never learns.
    pub by_hand: bool,
}

impl Row {
    /// What the row says.
    fn label(&self) -> (&str, Option<&str>) {
        match &self.name {
            Some(name) => (self.kind, Some(name.as_str())),
            None => (self.kind, None),
        }
    }
}

/// Which part of a row a press landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// The triangle: fold or unfold the subtree.
    Fold,
    /// The eye: hide the node here, or show it again.
    Eye,
    /// The row: select it, and maybe start dragging it.
    Body,
}

/// Where a press at `x` within a row of `width` landed.
pub fn hit(x: i32, width: i32, row: &Row) -> Hit {
    let fold = row.depth as i32 * INDENT;
    if row.parent && x >= fold && x < fold + FOLD {
        return Hit::Fold;
    }
    if x >= width - EYE {
        return Hit::Eye;
    }
    Hit::Body
}

/// A row being dragged, and where it would land.
#[derive(Clone, Debug)]
pub struct Drag {
    /// The node being dragged.
    pub path: Vec<usize>,
    /// Where the press went down.
    pub from: Point,
    /// Whether the pointer has travelled far enough to mean it.
    pub moved: bool,
    /// Where it would go: a parent, and an index among its children.
    pub onto: Option<(Vec<usize>, usize)>,
    /// Whether the drop would go *inside* the row under the pointer rather than
    /// beside it, for the marker to be drawn differently.
    pub into: bool,
}

/// The pane.
pub struct Outline {
    /// The panel every row is built in, replaced whenever anything changes.
    pub content: NodeId,
    /// The rows, top to bottom, as they are shown.
    pub rows: Vec<Row>,
    /// The field over a row being renamed.
    pub renaming: Option<(usize, NodeId)>,
}

/// What the pane needs in order to draw itself.
pub struct View<'a> {
    /// The rows to draw.
    pub rows: &'a [Row],
    /// Which of them are selected, by path.
    pub selection: &'a [Vec<usize>],
    /// A drag in progress, for the insertion marker.
    pub drag: Option<&'a Drag>,
    /// How wide the pane is.
    pub width: i32,
}

impl Outline {
    /// Builds the pane inside `parent`.
    pub fn build(ui: &mut Ui<Message>, parent: NodeId, view: View<'_>) -> Self {
        let width = view.width;
        let height = (view.rows.len() as i32 * ROW).max(ROW);
        let content = ui
            .add(parent, Panel::default(), Rect::new(0, 0, width, height))
            .expect("the outline's viewport is there");

        for (index, row) in view.rows.iter().enumerate() {
            let y = index as i32 * ROW;
            let selected = view.selection.contains(&row.path);
            let hidden = row.by_file || row.by_hand;

            if selected {
                ui.add(
                    content,
                    Panel::filled(Role::Primary),
                    Rect::new(0, y, width, ROW),
                );
            }

            let ink = match (selected, hidden) {
                (true, _) => Role::PrimaryContent,
                (false, true) => Role::Base300,
                (false, false) => Role::BaseContent,
            };
            let dim = if selected {
                Role::PrimaryContent
            } else {
                Role::Base300
            };

            // `-` and `+`, not a pair of triangles: the built-in 5x7 font
            // covers ASCII and Latin-1 and nothing else, so `▾` draws the
            // missing-character box. Which is what every tree control drew
            // before it had the glyphs for anything else.
            let mut x = row.depth as i32 * INDENT;
            if row.parent {
                ui.add(
                    content,
                    Label::new(if row.open { "-" } else { "+" })
                        .with_size(11)
                        .with_role(dim),
                    Rect::new(x + 2, y + 4, FOLD, 14),
                );
            }
            x += FOLD;

            // The kind, then the name: what it *is* and what it is *called*, in
            // that order, because a form full of panels is read by kind.
            let (kind, name) = row.label();
            let style = TextStyle::built_in(11);
            let taken = ui.text_mut().measure_line(style, kind) + GAP * 2;
            ui.add(
                content,
                Label::new(kind).with_size(11).with_role(dim),
                Rect::new(x, y + 4, taken, 14),
            );
            if let Some(name) = name {
                ui.add(
                    content,
                    Label::new(name).with_size(11).with_role(ink),
                    Rect::new(x + taken, y + 4, width - x - taken - EYE, 14),
                );
            }

            // `o` for what is drawn, `x` for what this pane has hidden, and `.`
            // for what the *file* hides — which the eye did not do and cannot
            // undo. Again ASCII, for the same reason as the fold.
            let (eye, why) = match (row.by_hand, row.by_file) {
                (true, _) => ("x", "hidden here; the file does not say so"),
                (false, true) => (".", "the file hides this, with visible=#false"),
                (false, false) => ("o", "hide this here, to reach what is behind it"),
            };
            if let Some(id) = ui.add(
                content,
                Label::new(eye).with_size(11).with_role(dim),
                Rect::new(width - EYE, y + 4, EYE, 14),
            ) {
                ui.set_tooltip(id, why);
            }
        }

        // The insertion marker: a line between two rows for a drop *beside*
        // something, and an outline round the row for a drop *inside* it.
        if let Some(drag) = view.drag.filter(|drag| drag.moved)
            && let Some((parent_path, index)) = &drag.onto
        {
            let at = marker_row(view.rows, parent_path, *index, drag.into);
            if drag.into {
                let outline = Panel {
                    fill: None,
                    border: Some(Role::Accent),
                    border_width: 1,
                    radius: denise::Radius::Box,
                    backdrop: false,
                };
                ui.add(content, outline, Rect::new(0, at * ROW, width, ROW));
            } else {
                ui.add(
                    content,
                    Panel::filled(Role::Accent),
                    Rect::new(0, at * ROW, width, 2),
                );
            }
        }

        Self {
            content,
            rows: view.rows.to_vec(),
            renaming: None,
        }
    }

    /// The row a path is on, if it is showing.
    pub fn row_of(&self, path: &[usize]) -> Option<usize> {
        self.rows.iter().position(|row| row.path == path)
    }
}

/// Which row a drop marker is drawn at.
///
/// For a drop *inside* a row it is that row. For one *beside* it, the line goes
/// above the row that would be pushed down — or under the last row of the
/// parent's subtree, when the drop is at the end.
fn marker_row(rows: &[Row], parent: &[usize], index: usize, into: bool) -> i32 {
    let mut child = parent.to_vec();
    child.push(index);
    if let Some(at) = rows.iter().position(|row| row.path == child) {
        return at as i32;
    }
    if into {
        return rows.iter().position(|row| row.path == parent).unwrap_or(0) as i32;
    }
    // Past the last child: under everything the parent holds.
    let last = rows
        .iter()
        .rposition(|row| row.path.starts_with(parent))
        .unwrap_or(0);
    last as i32 + 1
}

/// Every node, in file order, with the ones inside a folded parent left out.
///
/// `folded` and `hidden` are lists of paths rather than of anything sturdier,
/// which is worth knowing: an edit that shifts a path leaves them pointing at
/// whatever moved into its place. A form is small and a click puts it right, and
/// the alternative is a designer that has to give every node an identity the file
/// does not.
pub fn rows(
    placed: &[denise_forms::Placed],
    folded: &[Vec<usize>],
    hidden: &[Vec<usize>],
    by_file: impl Fn(&[usize]) -> bool,
) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::with_capacity(placed.len());
    for node in placed {
        // Inside something folded: not shown, and neither is anything below it.
        if folded
            .iter()
            .any(|shut| node.path.len() > shut.len() && node.path.starts_with(shut))
        {
            continue;
        }
        let parent = placed.iter().any(|other| {
            other.path.len() == node.path.len() + 1 && other.path.starts_with(&node.path)
        });
        rows.push(Row {
            path: node.path.clone(),
            kind: node.kind,
            name: node.name.clone(),
            depth: node.path.len() - 1,
            parent,
            open: !folded.contains(&node.path),
            by_file: by_file(&node.path),
            by_hand: hidden.contains(&node.path),
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placed(paths: &[&[usize]]) -> Vec<denise_forms::Placed> {
        paths
            .iter()
            .enumerate()
            .map(|(index, path)| denise_forms::Placed {
                id: denise_ui::NodeId::from_ffi(index as u64 + 1),
                parent: None,
                kind: "panel",
                name: None,
                path: path.to_vec(),
            })
            .collect()
    }

    #[test]
    fn folding_a_node_takes_everything_under_it_off_the_list() {
        let nodes = placed(&[&[0], &[1], &[1, 0], &[1, 0, 0], &[1, 1], &[2]]);
        let all = rows(&nodes, &[], &[], |_| false);
        assert_eq!(all.len(), 6);
        assert!(all[1].parent && all[1].open);

        let folded = rows(&nodes, &[vec![1]], &[], |_| false);
        let shown: Vec<&[usize]> = folded.iter().map(|row| row.path.as_slice()).collect();
        assert_eq!(shown, vec![[0].as_slice(), &[1], &[2]]);
        assert!(!folded[1].open, "it is drawn shut");
        assert!(folded[1].parent, "it still has children to unfold");
    }

    #[test]
    fn folding_something_deeper_leaves_what_is_above_it() {
        let nodes = placed(&[&[0], &[0, 0], &[0, 0, 0], &[0, 1]]);
        let folded = rows(&nodes, &[vec![0, 0]], &[], |_| false);
        let shown: Vec<&[usize]> = folded.iter().map(|row| row.path.as_slice()).collect();
        assert_eq!(shown, vec![[0].as_slice(), &[0, 0], &[0, 1]]);
    }

    #[test]
    fn depth_is_what_the_path_says_and_a_leaf_has_no_triangle() {
        let nodes = placed(&[&[0], &[0, 0], &[0, 0, 0]]);
        let all = rows(&nodes, &[], &[], |_| false);
        assert_eq!(
            all.iter().map(|row| row.depth).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert!(!all[2].parent, "a leaf has nothing to fold");
    }

    #[test]
    fn a_press_finds_the_triangle_the_eye_or_the_row() {
        let nodes = placed(&[&[0], &[0, 0]]);
        let all = rows(&nodes, &[], &[], |_| false);
        let (parent, leaf) = (&all[0], &all[1]);

        assert_eq!(hit(2, 224, parent), Hit::Fold);
        assert_eq!(hit(60, 224, parent), Hit::Body);
        assert_eq!(hit(220, 224, parent), Hit::Eye);
        // A leaf has no triangle, so the same press is the row.
        assert_eq!(hit(INDENT + 2, 224, leaf), Hit::Body);
        // And the triangle of a nested row is indented with it.
        let nested = Row {
            parent: true,
            ..leaf.clone()
        };
        assert_eq!(hit(INDENT + 2, 224, &nested), Hit::Fold);
        assert_eq!(hit(2, 224, &nested), Hit::Body);
    }
}
