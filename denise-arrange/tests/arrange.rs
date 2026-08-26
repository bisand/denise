//! Rectangles, and the arithmetic that produced them.
//!
//! Everything here builds a real tree, arranges it, and asks the tree where
//! things landed — rather than testing the arithmetic against itself. What the
//! crate promises is `Ui::set_layout` calls, so that is what is checked.

use denise::{Rect, Size, theme};
use denise_arrange::{Arrange, Flow, Sizing};
use denise_ui::widgets::{Alert, Label, Panel, Rating};
use denise_ui::{NodeId, Ui, Void};

fn ui() -> Ui<Void> {
    Ui::new(Size::new(400, 200), theme::DARK)
}

/// A panel with no opinion about its own size, to be placed.
fn block(ui: &mut Ui<Void>) -> NodeId {
    let root = ui.root();
    ui.add(root, Panel::default(), Rect::ZERO)
        .expect("the root takes a child")
}

#[test]
fn fixed_children_take_what_they_asked_for_and_the_gaps_go_between() {
    let mut ui = ui();
    let (a, b, c) = (block(&mut ui), block(&mut ui), block(&mut ui));

    let mut arrange = Arrange::new(Flow::Row);
    let row = arrange.root();
    arrange.set_gap(row, 10);
    for id in [a, b, c] {
        arrange.node(row, id, Sizing::Fixed(50));
    }
    arrange.apply(&mut ui, Rect::new(0, 0, 400, 40));

    assert_eq!(ui.layout(a), Some(Rect::new(0, 0, 50, 40)));
    assert_eq!(ui.layout(b), Some(Rect::new(60, 0, 50, 40)));
    assert_eq!(ui.layout(c), Some(Rect::new(120, 0, 50, 40)));
    // Not after the last one: that is what padding is for.
    assert_eq!(ui.layout(c).unwrap().right(), 170);
}

#[test]
fn flex_children_share_what_is_left_in_proportion() {
    let mut ui = ui();
    let (fixed, one, two) = (block(&mut ui), block(&mut ui), block(&mut ui));

    let mut arrange = Arrange::new(Flow::Row);
    let row = arrange.root();
    arrange.node(row, fixed, Sizing::Fixed(100));
    arrange.node(row, one, Sizing::Flex(1));
    arrange.node(row, two, Sizing::Flex(2));
    arrange.apply(&mut ui, Rect::new(0, 0, 400, 40));

    // 300 left over, shared 1:2.
    assert_eq!(ui.layout(one).unwrap().width, 100);
    assert_eq!(ui.layout(two).unwrap().width, 200);
    assert_eq!(ui.layout(two).unwrap().right(), 400, "did not end flush");
}

#[test]
fn the_rounding_goes_to_the_last_flex_child_so_the_row_ends_flush() {
    // Three equal shares of 100 is 33.33. Two get 33 and the last gets 34,
    // rather than the row ending a pixel short of its own edge.
    let mut ui = ui();
    let ids = [block(&mut ui), block(&mut ui), block(&mut ui)];

    let mut arrange = Arrange::new(Flow::Row);
    let row = arrange.root();
    for id in ids {
        arrange.node(row, id, Sizing::Flex(1));
    }
    arrange.apply(&mut ui, Rect::new(0, 0, 100, 40));

    let widths: Vec<i32> = ids.iter().map(|id| ui.layout(*id).unwrap().width).collect();
    assert_eq!(
        widths.iter().sum::<i32>(),
        100,
        "{widths:?} does not fill it"
    );
    assert_eq!(ui.layout(ids[2]).unwrap().right(), 100);
}

#[test]
fn a_hugging_label_is_as_wide_as_its_text() {
    // The case the whole thing exists for.
    let mut ui = ui();
    let root = ui.root();
    let short = ui.add(root, Label::new("Hi"), Rect::ZERO).unwrap();
    let long = ui
        .add(root, Label::new("Rather more of it"), Rect::ZERO)
        .unwrap();

    let mut arrange = Arrange::new(Flow::Column);
    let column = arrange.root();
    arrange.node(column, short, Sizing::Hug);
    arrange.node(column, long, Sizing::Hug);
    arrange.apply(&mut ui, Rect::new(0, 0, 400, 200));

    // In a column the main axis is the height, so both hug to one line and
    // both fill the width.
    assert_eq!(ui.layout(short).unwrap().width, 400);
    assert_eq!(
        ui.layout(short).unwrap().height,
        ui.layout(long).unwrap().height
    );

    // Turned on its side, the difference shows.
    let mut across = Arrange::new(Flow::Row);
    let row = across.root();
    across.node(row, short, Sizing::Hug);
    across.node(row, long, Sizing::Hug);
    across.apply(&mut ui, Rect::new(0, 0, 400, 40));
    assert!(
        ui.layout(long).unwrap().width > ui.layout(short).unwrap().width,
        "the longer label did not come out wider",
    );
}

#[test]
fn a_node_with_no_opinion_hugs_to_nothing_rather_than_to_something_invented() {
    let mut ui = ui();
    let panel = block(&mut ui);

    let mut arrange = Arrange::new(Flow::Row);
    let row = arrange.root();
    arrange.node(row, panel, Sizing::Hug);
    arrange.apply(&mut ui, Rect::new(0, 0, 400, 40));

    // Visibly nothing, which is the point: an invented width would have been
    // obeyed silently and looked almost right.
    assert_eq!(ui.layout(panel).unwrap().width, 0);
}

#[test]
fn a_hugging_child_is_measured_against_the_axis_the_container_can_promise() {
    // The two-pass claim, and why one pass is not enough. An alert has no
    // height until it knows the width its text wraps to, so in a column it must
    // be offered the column's width before it can answer.
    let mut ui = ui();
    let root = ui.root();
    let text = "A message quite long enough that it certainly wraps at two hundred pixels.";
    let narrow = ui
        .add(root, Alert::new(denise::Role::Info, text), Rect::ZERO)
        .unwrap();

    let mut arrange = Arrange::new(Flow::Column);
    let column = arrange.root();
    arrange.node(column, narrow, Sizing::Hug);

    arrange.apply(&mut ui, Rect::new(0, 0, 200, 400));
    let tall = ui.layout(narrow).unwrap().height;
    arrange.apply(&mut ui, Rect::new(0, 0, 600, 400));
    let short = ui.layout(narrow).unwrap().height;

    assert!(tall > short, "the alert did not wrap: {tall} vs {short}");
    assert!(short > 0, "it got no height at all");
}

#[test]
fn a_rating_hugs_against_the_height_it_is_given() {
    // The mirror: width for a height, in a row.
    let mut ui = ui();
    let root = ui.root();
    let stars = ui
        .add(root, Rating::<Void>::display(3.0), Rect::ZERO)
        .unwrap();

    let mut arrange = Arrange::new(Flow::Row);
    let row = arrange.root();
    arrange.node(row, stars, Sizing::Hug);

    arrange.apply(&mut ui, Rect::new(0, 0, 400, 20));
    let narrow = ui.layout(stars).unwrap().width;
    arrange.apply(&mut ui, Rect::new(0, 0, 400, 40));
    let wide = ui.layout(stars).unwrap().width;

    assert!(
        wide > narrow,
        "taller stars are wider stars: {wide} vs {narrow}"
    );
}

#[test]
fn padding_comes_off_every_side_before_anything_is_placed() {
    let mut ui = ui();
    let only = block(&mut ui);

    let mut arrange = Arrange::new(Flow::Row);
    let row = arrange.root();
    arrange.set_padding(row, 12);
    arrange.node(row, only, Sizing::Flex(1));
    arrange.apply(&mut ui, Rect::new(0, 0, 400, 40));

    assert_eq!(ui.layout(only), Some(Rect::new(12, 12, 376, 16)));
}

#[test]
fn a_nested_container_that_is_a_node_places_its_children_inside_itself() {
    // The coordinates a child of a node is placed in start at that node's
    // origin, which is what `Ui::set_layout` already means.
    let mut ui = ui();
    let root = ui.root();
    let bar = ui.add(root, Panel::default(), Rect::ZERO).unwrap();
    let inside = ui.add(bar, Panel::default(), Rect::ZERO).unwrap();
    let first = block(&mut ui);

    let mut arrange = Arrange::new(Flow::Column);
    let column = arrange.root();
    arrange.node(column, first, Sizing::Fixed(100));
    let group = arrange.group(column, Flow::Row, Sizing::Fixed(40), Some(bar));
    arrange.set_padding(group, 4);
    arrange.node(group, inside, Sizing::Flex(1));

    arrange.apply(&mut ui, Rect::new(0, 0, 400, 200));

    assert_eq!(ui.layout(bar), Some(Rect::new(0, 100, 400, 40)));
    // Relative to `bar`, not to the root.
    assert_eq!(ui.layout(inside), Some(Rect::new(4, 4, 392, 32)));
    // And absolutely, the tree agrees.
    assert_eq!(ui.bounds(inside), Some(Rect::new(4, 104, 392, 32)));
}

#[test]
fn a_group_with_no_node_of_its_own_is_only_a_grouping() {
    let mut ui = ui();
    let (a, b) = (block(&mut ui), block(&mut ui));

    let mut arrange = Arrange::new(Flow::Column);
    let column = arrange.root();
    let pair = arrange.group(column, Flow::Row, Sizing::Fixed(40), None);
    arrange.node(pair, a, Sizing::Flex(1));
    arrange.node(pair, b, Sizing::Flex(1));
    arrange.apply(&mut ui, Rect::new(0, 0, 400, 200));

    // No node took the group's rectangle, so its children are placed in the
    // same space the group was.
    assert_eq!(ui.layout(a), Some(Rect::new(0, 0, 200, 40)));
    assert_eq!(ui.layout(b), Some(Rect::new(200, 0, 200, 40)));
}

#[test]
fn a_hugging_group_is_as_big_as_what_it_holds() {
    let mut ui = ui();
    let root = ui.root();
    let one = ui.add(root, Label::new("One"), Rect::ZERO).unwrap();
    let two = ui.add(root, Label::new("Two"), Rect::ZERO).unwrap();
    let after = block(&mut ui);

    let mut arrange = Arrange::new(Flow::Column);
    let column = arrange.root();
    let group = arrange.group(column, Flow::Column, Sizing::Hug, None);
    arrange.set_gap(group, 6);
    arrange.node(group, one, Sizing::Hug);
    arrange.node(group, two, Sizing::Hug);
    arrange.node(column, after, Sizing::Fixed(20));
    arrange.apply(&mut ui, Rect::new(0, 0, 400, 200));

    let line = ui.layout(one).unwrap().height;
    // Two lines and the gap between them, and the next child starts after it.
    assert_eq!(ui.layout(after).unwrap().y, line * 2 + 6);
}

#[test]
fn a_layer_gives_every_child_the_whole_box() {
    let mut ui = ui();
    let (under, over) = (block(&mut ui), block(&mut ui));

    let mut arrange = Arrange::new(Flow::Layer);
    let layer = arrange.root();
    arrange.set_padding(layer, 5);
    arrange.node(layer, under, Sizing::Fixed(10));
    arrange.node(layer, over, Sizing::Flex(1));
    arrange.apply(&mut ui, Rect::new(0, 0, 100, 60));

    // Sizing is ignored on a layer: there is no main axis to divide.
    let whole = Rect::new(5, 5, 90, 50);
    assert_eq!(ui.layout(under), Some(whole));
    assert_eq!(ui.layout(over), Some(whole));
}

#[test]
fn a_container_with_no_room_left_places_children_at_nothing_rather_than_negative() {
    let mut ui = ui();
    let (a, b) = (block(&mut ui), block(&mut ui));

    let mut arrange = Arrange::new(Flow::Row);
    let row = arrange.root();
    arrange.set_padding(row, 40);
    arrange.node(row, a, Sizing::Fixed(500));
    arrange.node(row, b, Sizing::Flex(1));
    arrange.apply(&mut ui, Rect::new(0, 0, 60, 40));

    // The padding alone is wider than the rectangle. Nothing inverts, nothing
    // panics, and the flex child gets nothing because nothing is left.
    for id in [a, b] {
        let rect = ui.layout(id).expect("placed");
        assert!(rect.width >= 0 && rect.height >= 0, "{rect:?} inverted");
    }
    assert_eq!(ui.layout(b).unwrap().width, 0);
}

#[test]
fn applying_twice_is_the_same_as_applying_once() {
    // Nothing accumulates: `apply` is a function of the arrangement and the
    // rectangle, not of what the tree currently holds.
    let mut ui = ui();
    let root = ui.root();
    let label = ui.add(root, Label::new("Settings"), Rect::ZERO).unwrap();
    let rest = block(&mut ui);

    let mut arrange = Arrange::new(Flow::Row);
    let row = arrange.root();
    arrange.set_padding(row, 8);
    arrange.set_gap(row, 8);
    arrange.node(row, label, Sizing::Hug);
    arrange.node(row, rest, Sizing::Flex(1));

    arrange.apply(&mut ui, Rect::new(0, 0, 400, 44));
    let once = (ui.layout(label), ui.layout(rest));
    arrange.apply(&mut ui, Rect::new(0, 0, 400, 44));
    assert_eq!((ui.layout(label), ui.layout(rest)), once);
}

#[test]
fn an_empty_arrangement_does_nothing_and_does_not_panic() {
    let mut ui = ui();
    let arrange = Arrange::new(Flow::Row);
    arrange.apply(&mut ui, Rect::new(0, 0, 400, 40));
    arrange.apply(&mut ui, Rect::ZERO);
}
