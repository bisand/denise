//! Anchoring and docking, through the tree rather than through the arithmetic.
//!
//! [`denise_ui::anchor`]'s own unit tests pin the two rules as pure functions.
//! These pin what the tree does with them: that a resize reaches the nodes that
//! asked to follow it and none of the ones that did not, that docking shrinks the
//! box its siblings are placed in, that the damage covers what moved, and — the
//! one that matters most — that a tree which never mentions either behaves
//! exactly as it did before they existed.

use denise::{InputEvent, Point, Rect, Size, theme};
use denise_ui::widgets::{Button, Panel};
use denise_ui::{Anchors, Dock, NodeId, Ui, Void};

const START: Size = Size::new(200, 100);

fn tree() -> (Ui<Void>, NodeId) {
    let ui: Ui<Void> = Ui::new(START, theme::DARK);
    let root = ui.root();
    (ui, root)
}

fn resize(ui: &mut Ui<Void>, size: Size) {
    ui.handle(&[InputEvent::SurfaceResized {
        size,
        scale_factor: 1.0,
    }]);
}

fn panel(ui: &mut Ui<Void>, parent: NodeId, rect: Rect) -> NodeId {
    ui.add(parent, Panel::default(), rect).expect("added")
}

// ------------------------------------------------------------------ anchoring

#[test]
fn a_tree_that_never_mentions_anchoring_does_not_move() {
    let (mut ui, root) = tree();
    let rects = [
        Rect::new(0, 0, 50, 20),
        Rect::new(150, 80, 40, 15),
        Rect::new(90, 40, 20, 20),
    ];
    let ids: Vec<NodeId> = rects.iter().map(|&r| panel(&mut ui, root, r)).collect();

    for (id, rect) in ids.iter().zip(rects) {
        assert_eq!(ui.bounds(*id), Some(rect));
        assert_eq!(ui.anchors(*id), Anchors::TOP_LEFT);
    }

    // The whole compatibility claim: growing and shrinking the surface leaves
    // every one of them exactly where the application put it.
    for size in [Size::new(400, 300), Size::new(60, 40), START] {
        resize(&mut ui, size);
        for (id, rect) in ids.iter().zip(rects) {
            assert_eq!(ui.bounds(*id), Some(rect), "at {size:?}");
        }
    }
}

#[test]
fn a_right_anchored_node_follows_the_edge_it_asked_for() {
    let (mut ui, root) = tree();
    // 10 from the right edge, 20 wide.
    let follower = panel(&mut ui, root, Rect::new(170, 10, 20, 20));
    let stayer = panel(&mut ui, root, Rect::new(10, 10, 20, 20));
    ui.set_anchors(follower, Anchors::new(false, true, true, false));

    resize(&mut ui, Size::new(300, 100));

    let bounds = ui.bounds(follower).expect("laid out");
    assert_eq!(bounds, Rect::new(270, 10, 20, 20));
    assert_eq!(300 - (bounds.x + bounds.width), 10, "the gap it was given");
    assert_eq!(
        ui.bounds(stayer),
        Some(Rect::new(10, 10, 20, 20)),
        "its neighbour asked for nothing and got nothing"
    );
}

#[test]
fn a_node_held_at_both_ends_stretches_and_keeps_both_margins() {
    let (mut ui, root) = tree();
    let bar = panel(&mut ui, root, Rect::new(10, 10, 180, 20));
    ui.set_anchors(bar, Anchors::new(true, true, true, false));

    resize(&mut ui, Size::new(500, 100));
    assert_eq!(ui.bounds(bar), Some(Rect::new(10, 10, 480, 20)));

    // And back the other way, including past its own margins.
    resize(&mut ui, Size::new(15, 100));
    let squeezed = ui.bounds(bar).expect("laid out");
    assert_eq!(squeezed.width, 0, "never an inverted rectangle");
}

#[test]
fn anchoring_composes_through_a_nested_parent() {
    let (mut ui, root) = tree();
    let outer = panel(&mut ui, root, Rect::new(0, 0, 200, 100));
    let inner = panel(&mut ui, outer, Rect::new(170, 0, 20, 20));
    ui.set_anchors(outer, Anchors::STRETCH);
    ui.set_anchors(inner, Anchors::new(false, true, true, false));

    resize(&mut ui, Size::new(400, 100));

    assert_eq!(ui.bounds(outer), Some(Rect::new(0, 0, 400, 100)));
    assert_eq!(
        ui.bounds(inner),
        Some(Rect::new(370, 0, 20, 20)),
        "the child follows the parent that followed the surface"
    );
}

#[test]
fn a_resize_repaints_everything_and_says_so() {
    let (mut ui, root) = tree();
    let follower = panel(&mut ui, root, Rect::new(170, 10, 20, 20));
    ui.set_anchors(follower, Anchors::new(false, true, true, false));
    ui.presented();
    assert!(!ui.needs_paint(), "a settled tree asks for nothing");

    resize(&mut ui, Size::new(300, 100));

    // The surface itself changed size, so every pixel is new and the tree says
    // the whole thing is dirty — which `pending_damage` reports as an empty list
    // rather than as one big rectangle. Nothing subtler would be honest here.
    assert!(ui.needs_paint());
    assert!(
        ui.pending_damage().is_empty(),
        "a resize dirties the whole surface"
    );
}

#[test]
fn changing_an_anchor_damages_only_what_it_moved() {
    let (mut ui, root) = tree();
    let follower = panel(&mut ui, root, Rect::new(170, 10, 20, 20));
    let elsewhere = panel(&mut ui, root, Rect::new(0, 60, 20, 20));
    resize(&mut ui, Size::new(300, 100));
    ui.presented();
    assert!(!ui.needs_paint());

    // Anchoring it *now* moves it, because the surface already grew.
    ui.set_anchors(follower, Anchors::new(false, true, true, false));

    let damage: Vec<Rect> = ui.pending_damage().to_vec();
    assert!(!damage.is_empty(), "a targeted change, not a full repaint");
    let moved = ui.bounds(follower).expect("laid out");
    assert!(
        damage.iter().any(|r| r.intersect(&moved).is_some()),
        "where it went is not in the damage: {damage:?}"
    );
    assert!(
        damage
            .iter()
            .any(|r| r.intersect(&Rect::new(170, 10, 20, 20)).is_some()),
        "where it came from is not in the damage: {damage:?}"
    );
    assert!(
        !damage
            .iter()
            .any(|r| r.intersect(&ui.bounds(elsewhere).unwrap()).is_some()),
        "a node that did not move was repainted: {damage:?}"
    );
}

#[test]
fn setting_a_layout_re_baselines_the_anchor() {
    let (mut ui, root) = tree();
    let node = panel(&mut ui, root, Rect::new(170, 10, 20, 20));
    ui.set_anchors(node, Anchors::new(false, true, true, false));

    resize(&mut ui, Size::new(300, 100));
    assert_eq!(ui.bounds(node), Some(Rect::new(270, 10, 20, 20)));

    // A new layout is a new design, stated against the parent as it is now — so
    // it lands exactly where it was put, not where the old baseline implies.
    ui.set_layout(node, Rect::new(100, 10, 20, 20));
    assert_eq!(ui.bounds(node), Some(Rect::new(100, 10, 20, 20)));

    resize(&mut ui, Size::new(400, 100));
    assert_eq!(
        ui.bounds(node),
        Some(Rect::new(200, 10, 20, 20)),
        "and follows the edge from the new baseline"
    );
}

// -------------------------------------------------------------------- docking

#[test]
fn each_side_takes_its_edge_and_leaves_the_rest() {
    let (mut ui, root) = tree();
    let top = panel(&mut ui, root, Rect::new(0, 0, 0, 20));
    let bottom = panel(&mut ui, root, Rect::new(0, 0, 0, 10));
    let left = panel(&mut ui, root, Rect::new(0, 0, 30, 0));
    let body = panel(&mut ui, root, Rect::new(0, 0, 0, 0));

    ui.set_dock(top, Some(Dock::Top));
    ui.set_dock(bottom, Some(Dock::Bottom));
    ui.set_dock(left, Some(Dock::Left));
    ui.set_dock(body, Some(Dock::Fill));

    assert_eq!(ui.bounds(top), Some(Rect::new(0, 0, 200, 20)));
    assert_eq!(ui.bounds(bottom), Some(Rect::new(0, 90, 200, 10)));
    assert_eq!(ui.bounds(left), Some(Rect::new(0, 20, 30, 70)));
    assert_eq!(ui.bounds(body), Some(Rect::new(30, 20, 170, 70)));
    assert_eq!(ui.dock(body), Some(Dock::Fill));
}

#[test]
fn docking_follows_a_resize_without_anybody_being_anchored() {
    let (mut ui, root) = tree();
    let top = panel(&mut ui, root, Rect::new(0, 0, 0, 20));
    let body = panel(&mut ui, root, Rect::new(0, 0, 0, 0));
    ui.set_dock(top, Some(Dock::Top));
    ui.set_dock(body, Some(Dock::Fill));

    resize(&mut ui, Size::new(640, 480));

    assert_eq!(ui.bounds(top), Some(Rect::new(0, 0, 640, 20)));
    assert_eq!(ui.bounds(body), Some(Rect::new(0, 20, 640, 460)));
}

#[test]
fn a_docked_bar_moves_its_undocked_siblings_rather_than_covering_them() {
    let (mut ui, root) = tree();
    let body = panel(&mut ui, root, Rect::new(5, 5, 40, 40));
    assert_eq!(ui.bounds(body), Some(Rect::new(5, 5, 40, 40)));

    let bar = panel(&mut ui, root, Rect::new(0, 0, 0, 20));
    ui.set_dock(bar, Some(Dock::Top));

    assert_eq!(
        ui.bounds(body),
        Some(Rect::new(5, 25, 40, 40)),
        "an undocked sibling is placed in what the dock left"
    );
}

#[test]
fn a_hidden_dock_takes_no_room() {
    let (mut ui, root) = tree();
    let bar = panel(&mut ui, root, Rect::new(0, 0, 0, 20));
    let body = panel(&mut ui, root, Rect::new(0, 0, 0, 0));
    ui.set_dock(bar, Some(Dock::Top));
    ui.set_dock(body, Some(Dock::Fill));
    assert_eq!(ui.bounds(body), Some(Rect::new(0, 20, 200, 80)));

    ui.set_visible(bar, false);
    assert_eq!(
        ui.bounds(body),
        Some(Rect::new(0, 0, 200, 100)),
        "hiding a bar gives its room back, as hiding a stack child does"
    );
}

#[test]
fn undocking_gives_the_room_back() {
    let (mut ui, root) = tree();
    let bar = panel(&mut ui, root, Rect::new(0, 0, 0, 20));
    let body = panel(&mut ui, root, Rect::new(0, 0, 100, 50));
    ui.set_dock(bar, Some(Dock::Top));
    assert_eq!(ui.bounds(body), Some(Rect::new(0, 20, 100, 50)));

    ui.set_dock(bar, None);
    assert_eq!(ui.bounds(body), Some(Rect::new(0, 0, 100, 50)));
    assert_eq!(ui.dock(bar), None);
}

#[test]
fn a_stack_runs_inside_what_the_docks_left() {
    let (mut ui, root) = tree();
    let column = panel(&mut ui, root, Rect::new(0, 0, 200, 100));
    let bar = panel(&mut ui, column, Rect::new(0, 0, 0, 20));
    let first = panel(&mut ui, column, Rect::new(0, 0, 50, 10));
    let second = panel(&mut ui, column, Rect::new(0, 0, 50, 10));

    ui.set_dock(bar, Some(Dock::Top));
    ui.set_stack(column, 4);

    assert_eq!(ui.bounds(bar), Some(Rect::new(0, 0, 200, 20)));
    assert_eq!(
        ui.bounds(first),
        Some(Rect::new(0, 20, 50, 10)),
        "the stack starts below the bar, not under it"
    );
    assert_eq!(ui.bounds(second), Some(Rect::new(0, 34, 50, 10)));
}

#[test]
fn docking_survives_a_scrolled_parent() {
    let (mut ui, root) = tree();
    let view = panel(&mut ui, root, Rect::new(0, 0, 200, 100));
    ui.set_scrollable(view, true);
    let tall = panel(&mut ui, view, Rect::new(0, 0, 200, 400));
    ui.set_dock(tall, Some(Dock::Top));

    // The dock takes the viewport's height, not the content's, so there is
    // nothing to scroll — which is the honest answer and worth pinning.
    assert_eq!(ui.bounds(tall), Some(Rect::new(0, 0, 200, 100)));
}

#[test]
fn a_docked_node_is_hit_where_it_was_placed_and_not_where_it_was_written() {
    let (mut ui, root) = tree();
    // Its layout says the top-left corner; docking puts it along the bottom.
    let bar = ui
        .add(root, Button::<Void>::inert("bar"), Rect::new(0, 0, 10, 20))
        .expect("added");
    ui.set_dock(bar, Some(Dock::Bottom));
    assert_eq!(ui.bounds(bar), Some(Rect::new(0, 80, 200, 20)));

    // Hit testing reads what the reflow wrote, so it agrees with paint by
    // construction rather than by a second implementation staying in step.
    assert_eq!(ui.hit_test(Point::new(150, 90)), Some(bar));
    assert_eq!(
        ui.hit_test(Point::new(5, 5)),
        None,
        "nothing is where its layout was written"
    );
}
