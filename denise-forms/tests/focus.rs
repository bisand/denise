//! Where the caret starts, and where Tab goes next.
//!
//! Two promises a form makes about the keyboard, and [#98] asks for both to be
//! asserted rather than assumed:
//!
//! - `focus=#true` on one node is where the caret is when the form opens.
//! - **File order is tab order.** There is no `tab-index`, and there is not going
//!   to be one: the file already has an order, a person chose it, and the round
//!   trip keeps it ([#88]). A second number saying the same thing differently is
//!   a second thing to get out of step.
//!
//! [#98]: https://github.com/bisand/denise/issues/98
//! [#88]: https://github.com/bisand/denise/issues/88

use denise::{ElementState, InputEvent, KeyCode, Modifiers, Size};
use denise_forms::{Built, Form, Handler, Payload, Picture, Wiring};
use denise_ui::{NodeId, Ui, Void};

struct Anything;

impl Wiring<Void> for Anything {
    fn message(&mut self, _name: &str, payload: Payload) -> Option<Handler<Void>> {
        Some(match payload {
            Payload::None => Handler::Plain(Void),
            Payload::Bool => Handler::Bool(|_| Void),
            Payload::Index => Handler::Index(|_| Void),
            Payload::Number => Handler::Number(|_| Void),
        })
    }

    fn asset(&mut self, _path: &str) -> Option<Picture> {
        Some(Picture {
            pixels: vec![0xFF00_0000; 4],
            size: Size::new(2, 2),
        })
    }
}

fn build(source: &str) -> (Ui<Void>, Built) {
    let form = Form::parse(source).expect("the form parses");
    let mut ui: Ui<Void> = Ui::new(form.size(), form.theme());
    let root = ui.root();
    let built = form.build(&mut ui, root, &mut Anything).expect("it builds");
    (ui, built)
}

fn tab(ui: &mut Ui<Void>, backwards: bool) {
    ui.handle(&[InputEvent::Key {
        code: KeyCode::Tab,
        state: ElementState::Down,
        repeat: false,
        modifiers: if backwards {
            Modifiers::SHIFT
        } else {
            Modifiers::default()
        },
    }]);
}

/// The name the form gave whatever holds focus, or its kind.
fn here(ui: &Ui<Void>, built: &Built) -> String {
    let Some(id) = ui.focused() else {
        return String::from("<nothing>");
    };
    named(built, id).unwrap_or_else(|| format!("<{}>", ui.kind(id).unwrap_or("?")))
}

fn named(built: &Built, id: NodeId) -> Option<String> {
    built
        .names()
        .find(|(_, node)| *node == id)
        .map(|(name, _)| name.to_string())
}

/// Every focusable node, in the order Tab reaches them, starting from wherever
/// the form put the caret.
fn sequence(ui: &mut Ui<Void>, built: &Built) -> Vec<String> {
    let start = ui.focused();
    let mut seen: Vec<NodeId> = start.into_iter().collect();
    let mut out: Vec<String> = start
        .map(|id| named(built, id).unwrap_or_default())
        .into_iter()
        .collect();
    for _ in 0..200 {
        tab(ui, false);
        let Some(id) = ui.focused() else { break };
        if seen.contains(&id) {
            break;
        }
        seen.push(id);
        out.push(here(ui, built));
    }
    out
}

const THREE: &str = "\
form \"F\" version=1 width=300 height=300 {
    text-input name=first x=0 y=0 w=200 h=30
    button \"Second\" name=second x=0 y=40 w=200 h=30
    checkbox \"Third\" name=third x=0 y=80 w=200 h=30
}
";

#[test]
fn a_form_opens_with_the_caret_where_it_said() {
    let source = THREE.replace("name=first", "name=first focus=#true");
    let (ui, built) = build(&source);
    assert_eq!(here(&ui, &built), "first");

    // And with nothing said, nothing is focused: a form that grabbed the caret
    // without being asked would steal it from whatever put the form up.
    let (ui, built) = build(THREE);
    assert_eq!(here(&ui, &built), "<nothing>");
}

#[test]
fn tab_walks_the_file_in_the_order_the_file_is_written() {
    let source = THREE.replace("name=first", "name=first focus=#true");
    let (mut ui, built) = build(&source);
    assert_eq!(sequence(&mut ui, &built), ["first", "second", "third"]);
}

#[test]
fn moving_a_node_in_the_file_moves_it_in_the_tab_order() {
    // The whole convention, in one assertion: there is no `tab-index`, so this
    // is how a person changes the order — and it is what the designer's tab
    // order mode writes.
    let swapped = "\
form \"F\" version=1 width=300 height=300 {
    text-input name=first x=0 y=0 w=200 h=30 focus=#true
    checkbox \"Third\" name=third x=0 y=80 w=200 h=30
    button \"Second\" name=second x=0 y=40 w=200 h=30
}
";
    let (mut ui, built) = build(swapped);
    assert_eq!(sequence(&mut ui, &built), ["first", "third", "second"]);
}

#[test]
fn shift_tab_walks_it_backwards() {
    let source = THREE.replace("name=third", "name=third focus=#true");
    let (mut ui, built) = build(&source);
    assert_eq!(here(&ui, &built), "third");
    tab(&mut ui, true);
    assert_eq!(here(&ui, &built), "second");
    tab(&mut ui, true);
    assert_eq!(here(&ui, &built), "first");
    // And round, because a form is a loop rather than a line.
    tab(&mut ui, true);
    assert_eq!(here(&ui, &built), "third");
}

#[test]
fn a_node_that_cannot_take_focus_is_not_in_the_order() {
    let source = "\
form \"F\" version=1 width=300 height=300 {
    label \"Heading\" x=0 y=0 w=200 h=20
    text-input name=first x=0 y=30 w=200 h=30 focus=#true
    divider x=0 y=70 w=200 h=8
    button \"Go\" name=go x=0 y=90 w=200 h=30
}
";
    let (mut ui, built) = build(source);
    // The label and the divider are drawn and are not stops.
    assert_eq!(sequence(&mut ui, &built), ["first", "go"]);
}

#[test]
fn a_disabled_node_is_skipped_and_comes_back_when_it_is_enabled() {
    let source = "\
form \"F\" version=1 width=300 height=300 {
    text-input name=first x=0 y=0 w=200 h=30 focus=#true
    button \"Middle\" name=middle x=0 y=40 w=200 h=30 enabled=#false
    button \"Last\" name=last x=0 y=80 w=200 h=30
}
";
    let (mut ui, built) = build(source);
    assert_eq!(sequence(&mut ui, &built), ["first", "last"]);

    let middle = built.node("middle").expect("the form names it");
    ui.set_enabled(middle, true);
    ui.focus(built.node("first"));
    assert_eq!(sequence(&mut ui, &built), ["first", "middle", "last"]);
}

#[test]
fn a_nested_node_takes_its_parents_place_in_the_order() {
    // Depth first: everything inside a panel comes between the thing before the
    // panel and the thing after it. That is what makes "file order" a complete
    // answer rather than a per-parent one.
    let source = "\
form \"F\" version=1 width=300 height=300 {
    button \"Before\" name=before x=0 y=0 w=200 h=30 focus=#true
    panel name=card x=0 y=40 w=280 h=100 {
        text-input name=inside x=8 y=8 w=200 h=30
        button \"Also inside\" name=also x=8 y=48 w=200 h=30
    }
    button \"After\" name=after x=0 y=150 w=200 h=30
}
";
    let (mut ui, built) = build(source);
    assert_eq!(
        sequence(&mut ui, &built),
        ["before", "inside", "also", "after"],
    );
}

#[test]
fn z_moves_a_node_in_the_tab_order_as_well_as_in_front() {
    // A real coupling, and worth knowing about: `z=` sorts siblings, and the
    // tab order is the sibling order. Raising a node to the front therefore
    // makes it a later tab stop. This is deliberate — a thing drawn on top of
    // its siblings is reached after them — but it is the one way tab order
    // stops being *file* order, so it is asserted rather than discovered.
    let source = "\
form \"F\" version=1 width=300 height=300 {
    button \"First\" name=first x=0 y=0 w=200 h=30 focus=#true z=5
    button \"Second\" name=second x=0 y=40 w=200 h=30
}
";
    let (mut ui, built) = build(source);
    assert_eq!(sequence(&mut ui, &built), ["first", "second"]);
    // `first` is written first and raised, so it sorts last and Tab reaches it
    // last — the sequence read from `second` proves the order rather than the
    // starting point.
    ui.focus(built.node("second"));
    tab(&mut ui, false);
    assert_eq!(here(&ui, &built), "first");
}

#[test]
fn the_reference_form_tabs_through_every_stop_it_has_and_comes_back() {
    // The real one, headless. Twenty stops, and the sequence is the file read
    // top to bottom — starting at `full-name`, which is where the file puts the
    // caret, and wrapping round to it.
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../forms/reference.dform"
    ))
    .expect("the reference form");
    let (mut ui, built) = build(&source);

    assert_eq!(
        here(&ui, &built),
        "full-name",
        "the file says where to start"
    );

    let order = sequence(&mut ui, &built);
    assert_eq!(
        order,
        [
            // The rest of the form section, which is where the caret began.
            "full-name",
            "secret",
            "job",
            "delivery",
            "notify",
            "dark",
            "volume",
            "stars",
            "<button>", // Cancel
            "<button>", // Apply
            // Then the sections after it in the file.
            "records",
            "shots",
            // Not `retry`: the file gives it `no-focus=#true`, which is a
            // widget that takes no focus *and costs none* — a repeat button
            // beside a video is not somewhere Tab should stop.
            // Then round to the top: the header, the sidebar, the tabs.
            "<button>", // Docs
            "<button>", // Save
            "nav",
            "places",
            "advanced",
            "<checkbox>", // inside the collapse
            "<toggle>",   // inside the collapse
            "sections",
        ],
        "the tab order is no longer the file read top to bottom",
    );

    // `sequence` stops when Tab reaches somewhere it has already been, and
    // where it stopped is the start — because a form is a loop rather than a
    // line, and the twentieth stop leads back to the first.
    assert_eq!(here(&ui, &built), "full-name");
}
