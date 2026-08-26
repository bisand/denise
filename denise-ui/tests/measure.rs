//! What a widget says it would like to be, and the promises around the asking.
//!
//! The protocol is one trait method with a `None`-shaped default and one
//! `Ui::measure` that resolves a borrow. What makes it worth having is that the
//! answers are the same arithmetic the inherent `preferred_*` methods already
//! did — so these tests check the two against each other rather than restating
//! either — and that asking is free of everything except the theme and the
//! fonts.
//!
//! The caller this was written for is `docs/arrange.md`. Nothing in `denise-ui`
//! calls `measure`, which is the line between this toolkit and a layout engine.

use denise::{Rect, Size, theme};
use denise_ui::widgets::{
    Alert, Badge, Button, Image, Label, List, Panel, Rating, Select, Table, TextInput, Toggle,
    Tree, TreeItem, Video,
};
use denise_ui::{Measured, Offer, Ui, Void};

fn ui() -> Ui<Void> {
    Ui::new(Size::new(400, 300), theme::DARK)
}

/// Puts a widget in a tree and asks it, through the door this change opened.
fn measured(
    ui: &mut Ui<Void>,
    widget: impl denise_ui::Widget<Void> + 'static,
    offer: Offer,
) -> Measured {
    let root = ui.root();
    let id = ui
        .add(root, widget, Rect::new(0, 0, 10, 10))
        .expect("the root takes a child");
    ui.measure(id, offer)
}

#[test]
fn a_label_is_as_wide_as_its_text() {
    // The headline case, and the one that could not be answered at all before:
    // `Label` had no `preferred_width`, and "a label as wide as its text" is
    // what content-driven sizing means to anybody who asks for it.
    let mut ui = ui();
    let short = measured(&mut ui, Label::new("Hi"), Offer::NOTHING);
    let long = measured(
        &mut ui,
        Label::new("Hi, and rather more of it"),
        Offer::NOTHING,
    );

    let (short_w, long_w) = (
        short.width.expect("a label has a width"),
        long.width.expect("a label has a width"),
    );
    assert!(short_w > 0);
    assert!(long_w > short_w, "{long_w} is not wider than {short_w}");
    assert_eq!(short.height, long.height, "one line either way");
}

#[test]
fn measuring_does_not_depend_on_the_rectangle_it_currently_has() {
    // `MeasureCtx` carries no bounds, on purpose: a widget asked how big it
    // wants to be must not answer with how big it already is, or a layout pass
    // would converge on wherever it started.
    let mut ui = ui();
    let root = ui.root();
    let cramped = ui
        .add(root, Label::new("Nettverk"), Rect::new(0, 0, 1, 1))
        .expect("a child");
    let roomy = ui
        .add(root, Label::new("Nettverk"), Rect::new(0, 0, 900, 900))
        .expect("a child");

    assert_eq!(
        ui.measure(cramped, Offer::NOTHING),
        ui.measure(roomy, Offer::NOTHING)
    );
}

#[test]
fn a_widget_with_no_view_says_so_rather_than_inventing_one() {
    // A panel is the background other things sit on; an image and a video are
    // whatever rectangle they are given. `NOTHING` is the honest answer and the
    // caller decides — an invented size would be silently obeyed.
    let mut ui = ui();
    for (kind, got) in [
        ("panel", measured(&mut ui, Panel::default(), Offer::NOTHING)),
        (
            "image",
            measured(
                &mut ui,
                Image::new(vec![0; 4], Size::new(2, 2)),
                Offer::NOTHING,
            ),
        ),
        ("video", measured(&mut ui, Video::new(), Offer::NOTHING)),
    ] {
        assert_eq!(got, Measured::NOTHING, "{kind} invented a size");
    }
}

#[test]
fn an_alert_answers_a_height_only_when_it_is_told_a_width() {
    // Height for a width, because wrapped text has no height until it knows
    // what it wraps to. This is the whole reason arranging needs two passes.
    let mut ui = ui();
    let text = "A message long enough that it certainly wraps at two hundred pixels wide.";

    let unasked = measured(
        &mut ui,
        Alert::new(denise::Role::Info, text),
        Offer::NOTHING,
    );
    assert_eq!(
        unasked.height, None,
        "answered a height with no width to wrap to"
    );
    assert_eq!(unasked.width, None, "a banner is as wide as you make it");

    let narrow = measured(
        &mut ui,
        Alert::new(denise::Role::Info, text),
        Offer::wide(200),
    );
    let wide = measured(
        &mut ui,
        Alert::new(denise::Role::Info, text),
        Offer::wide(600),
    );
    let (narrow_h, wide_h) = (
        narrow.height.expect("a width was offered"),
        wide.height.expect("a width was offered"),
    );
    assert!(
        narrow_h > wide_h,
        "narrower should wrap taller: {narrow_h} vs {wide_h}"
    );
}

#[test]
fn a_rating_answers_a_width_only_when_it_is_told_a_height() {
    // The mirror of the alert, and the second witness that the shape is real
    // rather than one widget being awkward: stars are square.
    let mut ui = ui();
    let unasked = measured(&mut ui, Rating::<Void>::display(3.0), Offer::NOTHING);
    assert_eq!(unasked.width, None);

    let short = measured(&mut ui, Rating::<Void>::display(3.0), Offer::tall(20));
    let tall = measured(&mut ui, Rating::<Void>::display(3.0), Offer::tall(40));
    assert!(
        tall.width.expect("a height was offered") > short.width.expect("a height was offered"),
        "taller stars are wider stars",
    );
}

#[test]
fn the_protocol_reports_what_the_inherent_query_already_did() {
    // The claim that this is a door rather than a second copy of the
    // arithmetic. Each of these widgets had its own `preferred_*` before, and
    // `examples/gallery` still calls them; the two must not drift.
    let mut ui = ui();
    let mut engine = denise_text::TextEngine::new();
    let theme = theme::DARK;

    let badge = Badge::new("new");
    let want = Measured::both(
        badge.preferred_width(&mut engine),
        badge.preferred_height(&mut engine),
    );
    assert_eq!(measured(&mut ui, badge, Offer::NOTHING), want);

    let button = Button::<Void>::inert("Save");
    let width = button.preferred_width(&mut engine);
    assert_eq!(measured(&mut ui, button, Offer::NOTHING).width, Some(width));

    let list = List::<Void>::inert(["One", "Two", "Three"]);
    let want = Measured::both(
        list.preferred_width(&mut engine),
        list.preferred_height(&theme),
    );
    assert_eq!(measured(&mut ui, list, Offer::NOTHING), want);

    let toggle = Toggle::<Void>::inert("Dark theme");
    let width = toggle.preferred_width(&theme, &mut engine);
    assert_eq!(measured(&mut ui, toggle, Offer::NOTHING).width, Some(width));

    let tree = Tree::<Void>::inert([TreeItem::new("Branch"), TreeItem::new("Leaf").at_depth(1)]);
    let want = Measured::both(
        tree.preferred_width(&mut engine),
        tree.preferred_height(&theme),
    );
    assert_eq!(measured(&mut ui, tree, Offer::NOTHING), want);
}

#[test]
fn a_tree_that_folds_wants_to_be_shorter() {
    // The answer follows what is open, which is what makes it worth asking
    // again after a toggle rather than caching it forever.
    let mut ui = ui();
    let root = ui.root();
    let mut items = vec![TreeItem::new("Branch")];
    items.push(TreeItem::new("Leaf").at_depth(1));
    let id = ui
        .add(root, Tree::<Void>::inert(items), Rect::new(0, 0, 10, 10))
        .expect("a child");

    let open = ui.measure(id, Offer::NOTHING).height.expect("a height");
    ui.widget_mut::<Tree<Void>>(id)
        .expect("a tree")
        .set_open(0, false);
    let shut = ui.measure(id, Offer::NOTHING).height.expect("a height");

    assert!(
        shut < open,
        "folding did not make it shorter: {shut} vs {open}"
    );
}

#[test]
fn a_select_is_as_wide_as_the_option_it_has_to_show() {
    let mut ui = ui();
    let narrow = measured(
        &mut ui,
        Select::<Void>::new(["Av", "På"], Void),
        Offer::NOTHING,
    );
    let wide = measured(
        &mut ui,
        Select::<Void>::new(["Av", "En ganske lang valgmulighet"], Void),
        Offer::NOTHING,
    );
    assert!(
        wide.width.expect("a width") > narrow.width.expect("a width"),
        "a dropdown that clips the thing it exists to show",
    );
    assert_eq!(narrow.height, wide.height, "both are one field tall");
}

#[test]
fn a_field_has_a_height_and_no_view_about_its_width() {
    // What a field *is*: as wide as you make it, one line tall.
    let mut ui = ui();
    let got = measured(&mut ui, TextInput::<Void>::new(), Offer::NOTHING);
    assert_eq!(got.width, None);
    assert!(got.height.is_some_and(|h| h > 0));
}

#[test]
fn a_table_is_as_tall_as_its_rows_and_has_no_view_about_its_width() {
    let mut ui = ui();
    let one = measured(
        &mut ui,
        Table::<Void>::inert(["First", "Last"]).with_rows([["Ada", "Lovelace"]]),
        Offer::NOTHING,
    );
    let three = measured(
        &mut ui,
        Table::<Void>::inert(["First", "Last"]).with_rows([
            ["Ada", "Lovelace"],
            ["Grace", "Hopper"],
            ["Alan", "Turing"],
        ]),
        Offer::NOTHING,
    );
    assert!(three.height > one.height, "rows did not add height");
    // The columns divide what they are given; that is what `flex` means.
    assert_eq!(one.width, None);
}

#[test]
fn asking_about_a_node_that_is_not_there_is_not_an_error() {
    let mut ui = ui();
    let root = ui.root();
    let id = ui
        .add(root, Label::new("gone"), Rect::new(0, 0, 10, 10))
        .expect("a child");
    ui.remove(id);
    assert_eq!(ui.measure(id, Offer::NOTHING), Measured::NOTHING);
}
