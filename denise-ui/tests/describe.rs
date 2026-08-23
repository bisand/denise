//! Every widget describes itself, and the description is true.
//!
//! The point of [`Describe`] is that nothing outside a widget maintains a list of
//! its properties. That only holds if the list a widget publishes is the list it
//! actually honours, so these tests read `PROPERTIES` and then exercise every
//! entry in it rather than naming any property here. A widget that adds a setting
//! and forgets to handle it in `apply` fails; so does one that declares a range it
//! then clamps away from.

use denise::{Role, Size, theme};
use denise_ui::widgets::{
    Alert, Avatar, Badge, Button, Carousel, Checkbox, Collapse, Divider, DynDescribe, Image, Label,
    List, Mismatch, Panel, Progress, Property, PropertyKind, RadialProgress, RadioGroup, Rating,
    Select, Slider, Spinner, Table, Tabs, TextInput, Timeline, Toggle, Value, Video, all,
};
use denise_ui::{Ui, Void};

/// A message constructor for the one widget here whose constructor demands one,
/// in a tree whose messages are [`Void`].
fn void_bool(_: bool) -> Void {
    Void
}

/// A fresh widget of each kind, with enough content that an index property has
/// somewhere to point.
///
/// A widget built empty would clamp `selected=1` back to zero and the round trip
/// below would fail for a reason that has nothing to do with the description.
fn fresh(kind: &str) -> Box<dyn DynDescribe> {
    match kind {
        "alert" => Box::new(Alert::new(Role::Info, "Something happened")),
        "avatar" => Box::new(Avatar::initials("Ada Lovelace")),
        "badge" => Box::new(Badge::new("new")),
        "button" => Box::new(Button::<Void>::inert("Press")),
        "carousel" => Box::new(
            Carousel::<Void>::inert()
                .with_picture(vec![0; 4], Size::new(2, 2))
                .with_picture(vec![0; 4], Size::new(2, 2)),
        ),
        "checkbox" => Box::new(Checkbox::<Void>::inert("Tick")),
        "collapse" => Box::new(Collapse::new("Section", void_bool)),
        "divider" => Box::new(Divider::new()),
        "image" => Box::new(Image::new(vec![0; 4], Size::new(2, 2))),
        "label" => Box::new(Label::new("Text")),
        "list" => Box::new(List::<Void>::inert(["One", "Two", "Three"])),
        "panel" => Box::new(Panel::default()),
        "progress" => Box::new(Progress::new(0.0)),
        "radial-progress" => Box::new(RadialProgress::new(0.0)),
        "radio-group" => Box::new(RadioGroup::<Void>::inert(["One", "Two", "Three"])),
        "rating" => Box::new(Rating::<Void>::display(0.0)),
        "select" => Box::new(Select::<Void>::new(["One", "Two", "Three"], Void)),
        "slider" => Box::new(Slider::<Void>::inert(0.0, 100.0, 0.0)),
        "spinner" => Box::new(Spinner::new()),
        "table" => Box::new(
            Table::<Void>::inert(["First", "Last"])
                .with_rows([["Ada", "Lovelace"], ["Grace", "Hopper"]]),
        ),
        "tabs" => Box::new(Tabs::<Void>::inert(["One", "Two", "Three"])),
        "text-input" => Box::new(TextInput::<Void>::new()),
        "timeline" => Box::new(Timeline::new(["Started", "Finished"])),
        "toggle" => Box::new(Toggle::<Void>::inert("Switch")),
        "video" => Box::new(Video::new()),
        other => panic!(
            "the catalogue lists `{other}` and this test cannot build one. \
             Add it to `fresh`, which is the point: a widget nobody can construct \
             is a widget nobody can place."
        ),
    }
}

/// A value this property should accept, chosen from what it says it takes.
///
/// Small on purpose. A midpoint of an unbounded `Int` range would be an index no
/// list has, and the widget would clamp it — a failure about this function rather
/// than about the widget.
fn representative(property: &Property) -> Option<Value> {
    Some(match property.kind {
        PropertyKind::Text => Value::text("x"),
        PropertyKind::Color => Value::text("#11111B"),
        PropertyKind::Bool => Value::Bool(true),
        PropertyKind::Int { min, max } => Value::Int(1.clamp(min, max)),
        PropertyKind::Float { min, max } => Value::Float(0.5_f32.clamp(min, max)),
        // `none` on a role means "no fill", and a widget holding no fill reports
        // no value — a legitimate round trip that is not an equality, so this
        // takes the first name that stands for something.
        PropertyKind::Enum(names) => Value::Enum(names.iter().copied().find(|n| *n != "none")?),
        PropertyKind::Message(_) | PropertyKind::Asset => return None,
        // `PropertyKind` is `non_exhaustive`, so a kind added later reaches here.
        // Skipping it silently would let a new kind go untested; failing says so.
        other => panic!("this test has no representative value for {other:?}"),
    })
}

#[test]
fn every_settable_property_survives_being_set_and_read_back() {
    for widget in all() {
        for property in widget.properties {
            if !property.is_settable() {
                continue;
            }
            let Some(value) = representative(property) else {
                continue;
            };

            // A fresh widget per property, so setting one cannot disturb the
            // reading of another — a slider's step settling its value, a
            // rating's max re-clamping it.
            let mut subject = fresh(widget.kind);
            subject
                .set_property(property.name, value.clone())
                .unwrap_or_else(|error| {
                    panic!(
                        "{}.{} is declared but refused a {:?}: {error}",
                        widget.kind, property.name, value
                    )
                });

            let read = subject.get_property(property.name);
            assert_eq!(
                read,
                Some(value.clone()),
                "{}.{} did not read back what was set. \
                 Either `apply` ignores it, `get` reports something else, or the \
                 declared range allows a value the widget clamps away.",
                widget.kind,
                property.name
            );
        }
    }
}

#[test]
fn a_property_that_is_not_declared_is_refused_by_name() {
    for widget in all() {
        let mut subject = fresh(widget.kind);
        let error = subject
            .set_property("not-a-property", Value::text("x"))
            .expect_err("an undeclared property must be refused");

        assert_eq!(error.mismatch, Mismatch::Unknown, "on {}", widget.kind);
        let message = error.to_string();
        assert!(
            message.contains(widget.kind) && message.contains("not-a-property"),
            "the error must name the widget and the property: {message}"
        );
        for property in widget.properties {
            assert!(
                message.contains(property.name),
                "the error must list what {} does accept; {} is missing from: {message}",
                widget.kind,
                property.name
            );
        }
    }
}

#[test]
fn a_message_or_an_asset_says_so_rather_than_pretending() {
    for widget in all() {
        for property in widget.properties {
            if property.is_settable() {
                continue;
            }
            let mut subject = fresh(widget.kind);
            let error = subject
                .set_property(property.name, Value::text("greet"))
                .expect_err("a message or an asset cannot be set here");
            assert_eq!(
                error.mismatch,
                Mismatch::Supplied,
                "{}.{} must refuse with `Supplied`, since the engine wires it",
                widget.kind,
                property.name
            );
            assert_eq!(
                subject.get_property(property.name),
                None,
                "{}.{} must report nothing: the widget does not hold it",
                widget.kind,
                property.name
            );
        }
    }
}

#[test]
fn a_value_of_the_wrong_shape_is_refused() {
    for widget in all() {
        for property in widget.properties {
            if !property.is_settable() {
                continue;
            }
            // Text is the odd one out: everything that takes a string takes this,
            // and everything that does not should say so.
            let wrong = match property.kind {
                PropertyKind::Text => Value::Bool(true),
                // A string that is not a colour, so the widget must parse and refuse.
                PropertyKind::Color => Value::text("chartreuse"),
                _ => Value::text("definitely not"),
            };
            let mut subject = fresh(widget.kind);
            let error = subject
                .set_property(property.name, wrong)
                .expect_err("a value of the wrong shape must be refused");
            assert!(
                matches!(error.mismatch, Mismatch::WrongType { .. }),
                "{}.{} accepted a value of the wrong shape",
                widget.kind,
                property.name
            );
        }
    }
}

#[test]
fn the_catalogue_holds_every_widget_module() {
    // Read the module list out of the source rather than repeating it here, so
    // that adding a widget and forgetting the catalogue fails this test instead
    // of shipping a widget the designer's palette cannot show.
    const SOURCE: &str = include_str!("../src/widgets/mod.rs");

    // `style` is shared drawing helpers and `describe` is this machinery; neither
    // is a widget.
    const NOT_WIDGETS: [&str; 2] = ["style", "describe"];

    let modules: Vec<&str> = SOURCE
        .lines()
        .filter_map(|line| line.strip_prefix("mod ")?.strip_suffix(';'))
        .filter(|name| !NOT_WIDGETS.contains(name))
        .collect();

    assert!(
        modules.len() > 20,
        "the module list was not parsed; `mod.rs` must have changed shape: {modules:?}"
    );
    assert_eq!(
        modules.len(),
        all().len(),
        "there are {} widget modules and {} catalogue entries. \
         Every widget must appear in `describe::ALL`, or a palette will not show it.\n\
         modules: {modules:?}",
        modules.len(),
        all().len()
    );
}

#[test]
fn the_tree_reaches_a_widgets_properties_through_its_node() {
    let mut ui: Ui<Void> = Ui::new(Size::new(200, 100), theme::DARK);
    let root = ui.root();
    let id = ui
        .add(root, Label::new("before"), denise::Rect::new(0, 0, 100, 20))
        .expect("a label");

    assert_eq!(ui.kind(id), Some("label"));
    assert!(ui.properties(id).iter().any(|p| p.name == "text"));
    assert_eq!(ui.get_property(id, "text"), Some(Value::text("before")));

    ui.set_property(id, "text", Value::text("after"))
        .expect("the node describes itself")
        .expect("text takes a string");
    assert_eq!(ui.get_property(id, "text"), Some(Value::text("after")));

    // Setting a property must leave the node needing a repaint; a widget does not
    // own its damage, so nothing else would notice it changed.
    assert!(ui.needs_paint());
}

#[test]
fn a_widget_that_does_not_describe_itself_is_not_an_error() {
    let mut ui: Ui<Void> = Ui::new(Size::new(200, 100), theme::DARK);
    let root = ui.root();
    let id = ui
        .add(root, Void, denise::Rect::new(0, 0, 10, 10))
        .expect("a void node");

    assert_eq!(ui.kind(id), None);
    assert_eq!(ui.properties(id), &[]);
    assert_eq!(ui.get_property(id, "text"), None);
    assert!(ui.set_property(id, "text", Value::text("x")).is_none());
}
