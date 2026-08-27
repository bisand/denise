//! Building the repository's own form files, and every way one can be wrong.

use denise::{ElementState, InputEvent, KeyCode, Point, PointerButton, Size};
use denise_forms::{Error, Form, Handler, Payload, Picture, Reason, Wiring};
use denise_ui::widgets::TextInput;
use denise_ui::{Ui, Void};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Message {
    Greet,
}

/// A form file from the repository, read at run time.
///
/// Not `include_str!`: these live outside the crate, and a packaged
/// `denise-forms` must not depend on a path that only exists in the repository.
fn repo_form(name: &str) -> String {
    let path = format!("{}/../forms/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"))
}

/// Wiring that answers every name with whatever shape is asked for, and hands
/// back a picture for any path.
///
/// The engine's job is to ask for the right shape and to put what it gets in the
/// right place; what the application does with the name is the application's.
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

fn build_str<M: Clone + 'static>(
    source: &str,
    wiring: &mut impl Wiring<M>,
) -> Result<(Ui<M>, denise_forms::Built), Error> {
    let form = Form::parse(source)?;
    let mut ui: Ui<M> = Ui::new(form.size(), form.theme());
    let root = ui.root();
    let built = form.build(&mut ui, root, wiring)?;
    Ok((ui, built))
}

/// The error a source produces, or a panic naming what it built instead.
fn failure(source: &str) -> Reason {
    match build_str::<Void>(source, &mut Anything) {
        Err(error) => error.reason,
        Ok(_) => panic!("this should not have built:\n{source}"),
    }
}

// ----------------------------------------------------------------------- hello

#[test]
fn hello_dform_is_examples_hello() {
    let source = repo_form("hello.dform");
    let form = Form::parse(&source).expect("hello.dform parses");
    assert_eq!(form.title(), "Hello");
    assert_eq!(form.size(), Size::new(460, 260));
    assert_eq!(form.theme_name(), "dark");

    let mut ui: Ui<Message> = Ui::new(form.size(), form.theme());
    let root = ui.root();
    let built = form
        .build(&mut ui, root, &mut |name: &str, _: Payload| match name {
            "greet" => Some(Handler::Plain(Message::Greet)),
            _ => None,
        })
        .expect("hello.dform builds");

    let field = built.node("who").expect("the field is named");
    let greeting = built.node("greeting").expect("the label is named");

    // The file says `focus=#true`, so the first keystroke lands somewhere useful
    // — which is the thing `examples/hello` does in Rust on the line after it
    // builds its tree.
    assert_eq!(ui.focused(), Some(field));

    for ch in "Ada".chars() {
        ui.handle(&[InputEvent::Text { ch }]);
    }
    assert_eq!(
        ui.widget::<TextInput<Message>>(field).map(TextInput::text),
        Some("Ada")
    );

    // Enter submits, because the file said `on-submit=greet`.
    ui.handle(&[InputEvent::Key {
        code: KeyCode::Enter,
        state: ElementState::Down,
        repeat: false,
        modifiers: Default::default(),
    }]);
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Message::Greet]
    );

    // And so does the button, at the rectangle the file gave it.
    let button = ui.bounds(root).map(|_| Point::new(75, 145)).unwrap();
    ui.handle(&[
        InputEvent::PointerMoved { position: button },
        InputEvent::PointerButton {
            button: PointerButton::Left,
            state: ElementState::Down,
            position: button,
            modifiers: Default::default(),
        },
        InputEvent::PointerButton {
            button: PointerButton::Left,
            state: ElementState::Up,
            position: button,
            modifiers: Default::default(),
        },
    ]);
    assert_eq!(
        ui.drain_messages().collect::<Vec<_>>(),
        vec![Message::Greet]
    );
    assert!(ui.contains(greeting));
}

// ------------------------------------------------------------------- reference

#[test]
fn the_reference_form_builds_every_widget() {
    let source = repo_form("reference.dform");
    let (ui, built) = build_str::<Void>(&source, &mut Anything).expect("reference.dform builds");

    // Every widget kind the catalogue knows appears in that file by design, so a
    // widget added without the builder learning to construct it fails here.
    assert!(
        built.len() > 20,
        "the reference form names {} nodes",
        built.len()
    );
    for (name, id) in built.names() {
        assert!(ui.contains(id), "`{name}` is not in the tree");
    }
}

/// The layout the reference form produces, as text.
///
/// A committed *layout* snapshot rather than a rendered one, deliberately. A PPM
/// would be the more thorough check and would also be font-dependent: CI has no
/// TrueType font and a developer's machine does, so the two would disagree about
/// every pixel of text for reasons that are not regressions. Rectangles are what
/// this crate actually decides, and they are the same everywhere.
#[test]
fn the_reference_layout_is_what_it_was() {
    let source = repo_form("reference.dform");
    let (ui, built) = build_str::<Void>(&source, &mut Anything).expect("reference.dform builds");

    let mut lines: Vec<String> = built
        .names()
        .map(|(name, id)| {
            let kind = ui.kind(id).unwrap_or("?");
            let b = ui.bounds(id).expect("laid out");
            format!("{name} {kind} {} {} {} {}", b.x, b.y, b.width, b.height)
        })
        .collect();
    lines.sort();
    let actual = format!("{}\n", lines.join("\n"));

    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/reference.layout");
    if std::env::var_os("BLESS").is_some() {
        std::fs::write(path, &actual).expect("writing the snapshot");
        return;
    }
    let expected = std::fs::read_to_string(path).unwrap_or_default();
    assert_eq!(
        actual, expected,
        "the reference form lays out differently than it did. \
         If that is the intended change, re-run with BLESS=1 to record it."
    );
}

// ---------------------------------------------------------------------- errors

#[test]
fn a_file_that_is_not_kdl_says_where() {
    // An unterminated string is kdl's to complain about, and it does.
    let error = Form::parse("form \"x version=1").unwrap_err();
    assert!(matches!(error.reason, Reason::Syntax(_)), "{error}");
    assert!(error.to_string().starts_with('1'), "{error}");
}

/// A brace with no partner is named, and named before the parser runs.
///
/// The scan that counts depth counts balance for the same cost, and it beats
/// kdl to it with a better answer: the offending brace rather than wherever
/// the recovery gave up. It also matters more than tidiness — a file whose
/// braces do not balance is where kdl's exponential corners live (#104), so
/// this refusal is what keeps a malformed paste from costing minutes.
#[test]
fn a_brace_with_no_partner_is_named_and_pointed_at() {
    let error = Form::parse("form \"x\" version=1 {").unwrap_err();
    assert!(
        matches!(error.reason, Reason::Unbalanced { open: true }),
        "{error}"
    );
    // Column 20 is the `{` itself.
    assert!(error.to_string().starts_with("1:20"), "{error}");

    let error = Form::parse("form \"x\" version=1 {\n}\n}\n").unwrap_err();
    assert!(
        matches!(error.reason, Reason::Unbalanced { open: false }),
        "{error}"
    );
    assert!(error.to_string().starts_with("3:1"), "{error}");
}

#[test]
fn the_top_level_is_one_form_and_nothing_else() {
    assert!(matches!(
        Form::parse("").unwrap_err().reason,
        Reason::NotAForm { .. }
    ));
    assert!(matches!(
        Form::parse("window \"x\" version=1 width=1 height=1")
            .unwrap_err()
            .reason,
        Reason::NotAForm { .. }
    ));
    let two =
        Form::parse("form \"a\" version=1 width=1 height=1\nform \"b\" version=1 width=1 height=1")
            .unwrap_err();
    assert!(matches!(two.reason, Reason::NotAForm { .. }));
    assert_eq!(two.at.line, 2, "the second one is the problem: {two}");
}

#[test]
fn a_file_from_the_future_is_refused_by_number() {
    let error = Form::parse("form \"x\" version=99 width=1 height=1").unwrap_err();
    assert_eq!(
        error.reason,
        Reason::FromTheFuture {
            wanted: 99,
            understood: denise_forms::VERSION,
        }
    );
    assert!(error.to_string().contains("99"), "{error}");
}

#[test]
fn a_form_needs_a_version_and_a_size() {
    assert_eq!(
        Form::parse("form \"x\" width=1 height=1")
            .unwrap_err()
            .reason,
        Reason::Version
    );
    assert!(matches!(
        Form::parse("form \"x\" version=1 height=1")
            .unwrap_err()
            .reason,
        Reason::Missing { name: "width", .. }
    ));
}

#[test]
fn an_unknown_widget_lists_the_ones_there_are() {
    let reason = failure("form \"x\" version=1 width=9 height=9 { frobnicator x=0 y=0 w=1 h=1 }");
    assert!(matches!(reason, Reason::UnknownWidget { .. }));
    let message = build_str::<Void>(
        "form \"x\" version=1 width=9 height=9 { frobnicator x=0 y=0 w=1 h=1 }",
        &mut Anything,
    )
    .unwrap_err()
    .to_string();
    assert!(message.contains("frobnicator"), "{message}");
    assert!(message.contains("button"), "{message}");
}

#[test]
fn an_unknown_property_names_the_widget_and_what_it_accepts() {
    let error = build_str::<Void>(
        "form \"x\" version=1 width=9 height=9 {\n  button \"Go\" x=0 y=0 w=1 h=1 colour=red\n}",
        &mut Anything,
    )
    .unwrap_err();
    let Reason::UnknownProperty {
        kind, ref found, ..
    } = error.reason
    else {
        panic!("{error}");
    };
    assert_eq!(kind, "button");
    assert_eq!(found, "colour");
    let message = error.to_string();
    assert!(message.contains("button"), "{message}");
    assert!(message.contains("colour"), "{message}");
    assert!(message.contains("role"), "{message}");
    assert_eq!(error.at.line, 2, "{error}");
}

#[test]
fn a_property_given_the_wrong_shape_says_what_it_wanted() {
    let reason =
        failure("form \"x\" version=1 width=9 height=9 { label x=0 y=0 w=1 h=1 size=\"big\" }");
    let Reason::WrongType { kind, wanted, .. } = reason else {
        panic!("{reason:?}");
    };
    assert_eq!(kind, "label");
    assert_eq!(wanted, "a whole number");
}

#[test]
fn a_name_outside_its_table_lists_the_table() {
    let reason =
        failure("form \"x\" version=1 width=9 height=9 { label x=0 y=0 w=1 h=1 role=puce }");
    let Reason::NotAName {
        ref found,
        accepted,
        ..
    } = reason
    else {
        panic!("{reason:?}");
    };
    assert_eq!(found, "puce");
    assert!(accepted.contains(&"primary"));
}

#[test]
fn a_node_without_a_rectangle_is_refused() {
    for missing in ["x", "y", "w", "h"] {
        let mut props = String::new();
        for axis in ["x", "y", "w", "h"] {
            if axis != missing {
                props.push_str(&format!(" {axis}=1"));
            }
        }
        let source = format!("form \"x\" version=1 width=9 height=9 {{ label{props} }}");
        assert!(
            matches!(failure(&source), Reason::Missing { name, .. } if name == missing),
            "a label with no {missing} should say so"
        );
    }
}

#[test]
fn an_unknown_message_names_it_rather_than_going_quiet() {
    let source =
        "form \"x\" version=1 width=9 height=9 { button \"Go\" x=0 y=0 w=1 h=1 on-press=nope }";
    let form = Form::parse(source).unwrap();
    let mut ui: Ui<Message> = Ui::new(form.size(), form.theme());
    let root = ui.root();
    let error = form
        .build(&mut ui, root, &mut |_: &str, _: Payload| None)
        .unwrap_err();
    assert_eq!(
        error.reason,
        Reason::UnknownMessage {
            found: String::from("nope")
        }
    );
    assert!(error.to_string().contains("nope"), "{error}");
}

#[test]
fn a_message_of_the_wrong_shape_says_which_was_wanted() {
    let source =
        "form \"x\" version=1 width=9 height=9 { checkbox \"Tick\" x=0 y=0 w=1 h=1 on-change=go }";
    let form = Form::parse(source).unwrap();
    let mut ui: Ui<Message> = Ui::new(form.size(), form.theme());
    let root = ui.root();
    // A checkbox holds a `fn(bool) -> M`; this hands back a plain message.
    let error = form
        .build(&mut ui, root, &mut |_: &str, _: Payload| {
            Some(Handler::Plain(Message::Greet))
        })
        .unwrap_err();
    let Reason::WrongMessage { wanted, .. } = error.reason else {
        panic!("{error}");
    };
    assert!(wanted.contains("bool"), "{wanted}");
}

#[test]
fn a_picture_nobody_can_load_names_the_path() {
    // The default `Wiring` has no assets at all, which is the case an application
    // that forgot to supply a loader actually hits.
    let source = "form \"x\" version=1 width=9 height=9 { image x=0 y=0 w=1 h=1 src=\"logo.png\" }";
    let form = Form::parse(source).unwrap();
    let mut ui: Ui<Void> = Ui::new(form.size(), form.theme());
    let root = ui.root();
    let error = form
        .build(&mut ui, root, &mut |_: &str, _: Payload| {
            Some(Handler::Plain(Void))
        })
        .unwrap_err();
    assert_eq!(
        error.reason,
        Reason::Asset {
            path: String::from("logo.png")
        }
    );
}

#[test]
fn two_nodes_cannot_share_a_name() {
    let reason = failure(
        "form \"x\" version=1 width=9 height=9 {\n  label name=a x=0 y=0 w=1 h=1\n  label name=a x=0 y=2 w=1 h=1\n}",
    );
    assert!(matches!(reason, Reason::DuplicateName { .. }), "{reason:?}");
}

#[test]
fn only_one_node_may_hold_the_caret() {
    let reason = failure(
        "form \"x\" version=1 width=9 height=9 {\n  text-input x=0 y=0 w=1 h=1 focus=#true\n  text-input x=0 y=2 w=1 h=1 focus=#true\n}",
    );
    assert_eq!(reason, Reason::TwoFocuses);
}

#[test]
fn a_widget_that_cannot_hold_children_says_so() {
    let reason = failure(
        "form \"x\" version=1 width=9 height=9 { label x=0 y=0 w=1 h=1 { label x=0 y=0 w=1 h=1 } }",
    );
    let Reason::UnexpectedChild { ref parent, .. } = reason else {
        panic!("{reason:?}");
    };
    assert_eq!(parent, "label");
}

#[test]
fn a_collection_node_outside_its_parent_is_not_silently_dropped() {
    let reason = failure("form \"x\" version=1 width=9 height=9 { option \"one\" }");
    assert!(
        matches!(reason, Reason::UnexpectedChild { .. }),
        "{reason:?}"
    );
}

#[test]
fn a_select_and_a_collapse_build_without_a_message_and_say_nothing() {
    // Both used to require one, because neither had an inert constructor: a
    // form file cannot invent a plain message or a `fn(bool) -> M`. #118 gave
    // them one each, so a decorative section and a display-only dropdown no
    // longer have to name a message nobody wanted.
    let source = concat!(
        "form \"x\" version=1 width=200 height=200 {\n",
        "    select name=chosen x=0 y=0 w=120 h=24 { option \"one\"; option \"two\" }\n",
        "    collapse \"Section\" name=fold x=0 y=40 w=180 h=120\n",
        "}\n",
    );
    let (mut ui, built) = build_str::<Void>(source, &mut Anything).expect("it builds");

    let select = built.node("chosen").expect("the select is named");
    let fold = built.node("fold").expect("the collapse is named");

    // A press on each, and nothing comes back: there is no message to come.
    for id in [select, fold] {
        let bounds = ui.bounds(id).expect("bounds");
        let at = Point::new(bounds.x + 4, bounds.y + 4);
        ui.handle(&press_at(at));
        ui.handle(&[release_at(at)]);
    }
    let fired: Vec<Void> = ui.drain_messages().collect();
    assert!(fired.is_empty(), "an inert widget emitted something");

    // The section really folds, rather than only flipping its chevron: nothing
    // else is going to drive it. See `Collapse::inert`.
    let tall = ui.bounds(fold).expect("bounds").height;
    for now in [0, 100, 200, 400] {
        ui.tick(now);
    }
    let folded = ui.bounds(fold).expect("bounds").height;
    assert!(
        folded < tall,
        "an inert collapse did not fold: {tall} then {folded}"
    );

    // And opening it again returns to exactly where it was.
    let at = {
        let bounds = ui.bounds(fold).expect("bounds");
        Point::new(bounds.x + 4, bounds.y + 4)
    };
    ui.handle(&press_at(at));
    ui.handle(&[release_at(at)]);
    for now in [500, 600, 700, 900] {
        ui.tick(now);
    }
    assert_eq!(
        ui.bounds(fold).expect("bounds").height,
        tall,
        "opening it again did not return to the height it folded from"
    );
}

/// A press, as two events.
fn press_at(at: Point) -> [InputEvent; 2] {
    [
        InputEvent::PointerMoved { position: at },
        InputEvent::PointerButton {
            button: PointerButton::Left,
            state: ElementState::Down,
            position: at,
            modifiers: denise::Modifiers::NONE,
        },
    ]
}

fn release_at(at: Point) -> InputEvent {
    InputEvent::PointerButton {
        button: PointerButton::Left,
        state: ElementState::Up,
        position: at,
        modifiers: denise::Modifiers::NONE,
    }
}

#[test]
fn a_form_deeper_than_the_limit_is_refused_rather_than_overflowing() {
    let mut source = String::from("form \"x\" version=1 width=9 height=9 {");
    let depth = 200;
    for _ in 0..depth {
        source.push_str(" panel x=0 y=0 w=9 h=9 {");
    }
    for _ in 0..=depth {
        source.push('}');
    }
    assert!(matches!(failure(&source), Reason::TooDeep { .. }));
}

/// Commented-out blocks nested inside one another never reach `kdl`.
///
/// This test proves itself by finishing. `kdl` 6.7.1 takes time that doubles
/// with every level of this shape, so the sixty-four below would take longer
/// than the age of the universe if the guard let them through — a regression
/// here does not fail the assertion, it hangs the suite, which is the loudest
/// a test can be. Found by the fuzz target `parse_form`.
#[test]
fn a_form_that_would_take_forever_to_parse_is_refused_before_parsing() {
    let deep = denise_forms::MAX_DEPTH;
    let source = format!(
        "form \"x\" version=1 width=9 height=9 {{\n{}{}\n}}",
        "/- panel {\n".repeat(deep),
        "}".repeat(deep),
    );
    let error = Form::parse(&source).expect_err("a shape kdl cannot be asked to read");
    assert!(
        matches!(
            error.reason,
            Reason::CommentedTooDeep { limit } if limit == denise_forms::MAX_COMMENTED_DEPTH
        ),
        "{:?}",
        error.reason
    );
    // One level of it is a person taking a widget out for a minute, and that
    // still opens.
    let fine = "form \"x\" version=1 width=99 height=99 {\n    \
        /- panel name=box x=1 y=1 w=9 h=9 {\n        label \"gone\" x=1 y=1 w=5 h=5\n    }\n\
        \n    label \"here\" x=1 y=20 w=50 h=9\n}\n";
    let form = Form::parse(fine).expect("one commented-out block is ordinary");
    assert_eq!(form.text(), fine);
}

/// A file over the size limit is an error before it is a parse.
///
/// The guard fires on the byte count, ahead of the parser — a pathological
/// file must cost an `Error` rather than however long a 100 MB parse takes.
/// The fuzz target `parse_form` asserts the same limit from the other side:
/// nothing larger may *succeed*.
#[test]
fn a_form_larger_than_the_limit_is_refused_by_size() {
    let mut source = String::from("form \"x\" version=1 width=9 height=9 {\n");
    let filler = "    label \"padding padding padding\" x=0 y=0 w=1 h=1\n";
    while source.len() <= denise_forms::MAX_SOURCE {
        source.push_str(filler);
    }
    source.push('}');
    let error = Form::parse(&source).expect_err("over the limit");
    assert!(
        matches!(error.reason, Reason::TooLarge { limit } if limit == denise_forms::MAX_SOURCE),
        "{:?}",
        error.reason
    );
    // And it points at 1:1 rather than at a span the parser never produced,
    // because nothing was parsed at all.
    assert!(error.to_string().starts_with("1:1"), "{error}");
}

// ------------------------------------------------------------- what it applies

#[test]
fn properties_are_applied_in_descriptor_order_not_file_order() {
    // A slider clamps `value` into its range, so a file that writes `value`
    // before `min` and `max` must still land on 70 — descriptor order is what
    // makes two files that say the same thing build the same tree.
    let backwards = "form \"x\" version=1 width=99 height=99 { slider name=s x=0 y=0 w=50 h=10 value=70 max=100 min=0 }";
    let forwards = "form \"x\" version=1 width=99 height=99 { slider name=s x=0 y=0 w=50 h=10 min=0 max=100 value=70 }";

    let read = |source: &str| {
        let (ui, built) = build_str::<Void>(source, &mut Anything).expect("builds");
        let id = built.node("s").expect("named");
        ui.get_property(id, "value")
    };
    assert_eq!(read(backwards), read(forwards));
    assert_eq!(read(forwards), Some(denise_ui::widgets::Value::Float(70.0)));
}

#[test]
fn a_selection_survives_the_collection_it_points_into() {
    // The options are child nodes given to the constructor, and `selected` is a
    // property applied afterwards — so the index is clamped against a list that
    // already exists rather than against an empty one.
    let source = "form \"x\" version=1 width=99 height=99 {
        radio-group name=r x=0 y=0 w=90 h=60 selected=2 on-change=pick {
            option \"One\"
            option \"Two\"
            option \"Three\"
        }
    }";
    let (ui, built) = build_str::<Void>(source, &mut Anything).expect("builds");
    let id = built.node("r").expect("named");
    assert_eq!(
        ui.get_property(id, "selected"),
        Some(denise_ui::widgets::Value::Int(2))
    );
}

#[test]
fn the_tree_properties_reach_the_tree() {
    let source = "form \"x\" version=1 width=200 height=100 {
        panel name=bar x=0 y=0 w=0 h=20 dock=top
        panel name=body x=10 y=30 w=50 h=20 anchor=\"left top right\" z=3 tooltip=\"the body\"
        label name=gone x=0 y=0 w=5 h=5 visible=#false
        label name=off x=0 y=0 w=5 h=5 enabled=#false
    }";
    let (ui, built) = build_str::<Void>(source, &mut Anything).expect("builds");

    let bar = built.node("bar").unwrap();
    assert_eq!(ui.dock(bar), Some(denise_ui::Dock::Top));
    assert_eq!(ui.bounds(bar).unwrap(), denise::Rect::new(0, 0, 200, 20));

    let body = built.node("body").unwrap();
    assert_eq!(
        ui.anchors(body),
        denise_ui::Anchors::new(true, true, true, false)
    );
    // Placed in what the dock left, so its y is 30 past the bar's 20.
    assert_eq!(ui.bounds(body).unwrap().y, 50);

    assert!(ui.contains(built.node("gone").unwrap()));
    assert!(ui.contains(built.node("off").unwrap()));
}

#[test]
fn an_anchor_edge_that_is_not_an_edge_is_refused() {
    let reason = failure(
        "form \"x\" version=1 width=9 height=9 { label x=0 y=0 w=1 h=1 anchor=\"left sideways\" }",
    );
    let Reason::NotAName { ref found, .. } = reason else {
        panic!("{reason:?}");
    };
    assert_eq!(found, "sideways");
}

// ----------------------------------------------------------------- editing

/// The lines that differ between two versions of a file.
fn changed_lines(before: &str, after: &str) -> Vec<(String, String)> {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    assert_eq!(a.len(), b.len(), "an edit changed how many lines there are");
    a.iter()
        .zip(&b)
        .filter(|(x, y)| x != y)
        .map(|(x, y)| ((*x).to_string(), (*y).to_string()))
        .collect()
}

#[test]
fn moving_a_node_is_a_one_line_diff() {
    let source = repo_form("reference.dform");
    let mut form = Form::parse(&source).expect("parses");

    // The slider, four levels into the file and inside a panel.
    let (ui, built) = build_str::<Void>(&source, &mut Anything).expect("builds");
    let id = built.node("volume").expect("named");
    let path = built
        .placed()
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.path.clone())
        .expect("placed");
    drop(ui);

    assert!(form.set_number(&path, "y", 400));

    let after = form.text();
    let diff = changed_lines(&source, &after);
    assert_eq!(
        diff.len(),
        1,
        "a move touched {} lines: {diff:#?}",
        diff.len()
    );
    assert!(diff[0].0.contains("y=388"), "{:?}", diff[0]);
    assert!(diff[0].1.contains("y=400"), "{:?}", diff[0]);
    // Everything else on that line came along unchanged.
    assert!(diff[0].1.contains("name=volume"), "{:?}", diff[0]);
    assert!(diff[0].1.contains("on-change=set-volume"), "{:?}", diff[0]);

    // And it still loads.
    Form::parse(&after).expect("an edited form is still a form");
}

#[test]
fn an_untouched_form_comes_back_byte_for_byte() {
    let source = repo_form("reference.dform");
    let form = Form::parse(&source).expect("parses");
    assert_eq!(form.text(), source);
}

#[test]
fn a_property_that_was_not_there_is_appended_rather_than_refused() {
    let mut form = Form::parse(
        "form \"F\" version=1 width=99 height=99 {\n    label \"hi\" x=1 y=2 w=3 h=4\n}\n",
    )
    .expect("parses");
    assert!(form.set_number(&[0], "z", 5));
    assert!(form.text().contains("z=5"), "{}", form.text());
    assert!(form.text().contains("x=1 y=2 w=3 h=4"), "{}", form.text());
}

#[test]
fn clearing_a_property_takes_it_out_of_the_file() {
    let mut form = Form::parse(
        "form \"F\" version=1 width=99 height=99 {\n    label \"hi\" x=1 y=2 w=3 h=4 z=9\n}\n",
    )
    .expect("parses");
    assert!(form.clear_property(&[0], "z"));
    assert!(!form.text().contains("z=9"), "{}", form.text());
    assert!(
        !form.clear_property(&[0], "z"),
        "clearing twice is not an error"
    );
}

#[test]
fn removing_a_node_takes_its_children_with_it() {
    let mut form = Form::parse(
        "form \"F\" version=1 width=99 height=99 {\n\
         \x20   panel name=p x=0 y=0 w=9 h=9 {\n\
         \x20       label \"inside\" x=0 y=0 w=1 h=1\n\
         \x20   }\n\
         \x20   label \"after\" x=0 y=20 w=1 h=1\n\
         }\n",
    )
    .expect("parses");

    assert!(form.remove_at(&[0]));
    let after = form.text();
    assert!(!after.contains("inside"), "{after}");
    assert!(!after.contains("name=p"), "{after}");
    assert!(after.contains("after"), "the sibling went too: {after}");
    Form::parse(&after).expect("still a form");

    assert!(!form.remove_at(&[9]), "a path to nothing is not a removal");
}

#[test]
fn every_node_is_placed_with_a_path_that_points_back_at_it() {
    let source = repo_form("reference.dform");
    let (_ui, built) = build_str::<Void>(&source, &mut Anything).expect("builds");

    // More than the named ones: a designer selects what was clicked, and most of
    // what gets clicked was never named.
    assert!(
        built.placed().len() > built.len(),
        "{} placed against {} named",
        built.placed().len(),
        built.len()
    );

    let mut form = Form::parse(&source).expect("parses");
    for placed in built.placed() {
        assert!(
            form.set_number(&placed.path, "z", 0),
            "the path for `{}` ({}) points at nothing: {:?}",
            placed.name.as_deref().unwrap_or("unnamed"),
            placed.kind,
            placed.path
        );
    }

    // A node under the form has a one-element path; a node inside a panel has
    // more, and its parent is the panel.
    let nested = built
        .placed()
        .iter()
        .find(|p| p.name.as_deref() == Some("volume"))
        .expect("the slider");
    assert!(nested.path.len() > 1, "{:?}", nested.path);
    assert!(nested.parent.is_some());
    assert_eq!(nested.kind, "slider");
}

// ----------------------------------------------------------------- seeding

/// Every widget a designer could drop, dropped.
///
/// The test that makes [`denise_forms::seed`] impossible to let drift: a
/// twenty-sixth widget with a property the builder requires fails here until
/// `seed` learns to give it one, rather than failing in a designer that places
/// the widget and breaks the form.
#[test]
fn every_widget_that_ships_seeds_a_node_that_builds() {
    for widget in denise_ui::widgets::all() {
        let size = denise_forms::default_size(widget.kind);
        assert!(
            size.width > 0 && size.height > 0,
            "`{}` starts out with no rectangle",
            widget.kind
        );

        let rect = denise::Rect::new(8, 16, size.width as i32, size.height as i32);
        let node = denise_forms::seed(widget.kind, rect);
        let source = format!("form \"S\" version=1 width=400 height=300 {{\n    {node}\n}}\n");

        let (_, built) = build_str::<Void>(&source, &mut Anything).unwrap_or_else(|error| {
            panic!(
                "`{}` seeded a node that will not build: {error}\n{node}",
                widget.kind
            )
        });
        let placed = built
            .at(&[0])
            .unwrap_or_else(|| panic!("`{}` built nothing", widget.kind));
        assert_eq!(placed.kind, widget.kind);
    }
}

#[test]
fn a_seeded_node_lands_where_it_was_dropped_and_says_nothing_else() {
    let rect = denise::Rect::new(40, 60, 100, 32);
    let node = denise_forms::seed("button", rect);
    assert_eq!(node, r#"button "button" x=40 y=60 w=100 h=32"#);

    // A rect and nothing else, wherever a rect is enough.
    assert_eq!(
        denise_forms::seed("panel", rect),
        "panel x=40 y=60 w=100 h=32"
    );
    // And the least more than that, where it is not: without a range a slider
    // parses and then will not build.
    assert_eq!(
        denise_forms::seed("slider", rect),
        "slider x=40 y=60 w=100 h=32 min=0 max=100"
    );
}

#[test]
fn only_the_two_kinds_that_lay_children_out_own_children() {
    assert!(denise_forms::owns_children("panel"));
    assert!(denise_forms::owns_children("collapse"));
    // Content is not children: a `select` holds options, and dropping a button
    // on one has missed.
    for kind in ["select", "table", "carousel", "list", "label", "tabs"] {
        assert!(
            !denise_forms::owns_children(kind),
            "`{kind}` claimed children"
        );
    }
}

// ------------------------------------------------------- the form's own kind

/// A form node with whatever is given after the required parts.
fn form_with(extra: &str) -> String {
    format!("form \"F\" version=1 width=400 height=300 {extra}\n")
}

#[test]
fn a_property_the_form_node_does_not_have_is_a_mistake_like_any_other() {
    // The one place a typo used to go quietly into the file and stay there.
    let reason = Form::parse(&form_with("widht=800")).unwrap_err().reason;
    match reason {
        Reason::UnknownFormProperty { found, kind, .. } => {
            assert_eq!(found, "widht");
            assert_eq!(kind, "screen");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_kinds_own_property_is_accepted_on_it_and_refused_on_every_other_kind() {
    for (kind, property) in [
        ("window", "resizable=#true"),
        ("window", "min-width=320"),
        ("dialog", "dim=200"),
        ("drawer", "side=after"),
    ] {
        let extent = if kind == "drawer" { " extent=200" } else { "" };
        let good = form_with(&format!("kind={kind} {property}{extent}"));
        Form::parse(&good).unwrap_or_else(|error| panic!("`{property}` on {kind}: {error}"));

        // And on a screen, which has none of them, it says whose it is.
        let bad = form_with(&format!("kind=screen {property}"));
        let error = Form::parse(&bad).unwrap_err();
        assert!(
            matches!(error.reason, Reason::UnknownFormProperty { .. }),
            "`{property}` was accepted on a screen: {error}"
        );
        assert!(error.to_string().contains("screen"), "{error}");
    }
}

#[test]
fn what_comes_in_from_an_edge_has_to_say_how_far() {
    for kind in ["drawer", "shelf"] {
        let error = Form::parse(&form_with(&format!("kind={kind}"))).unwrap_err();
        match &error.reason {
            Reason::Missing { name, .. } => assert_eq!(*name, "extent"),
            other => panic!("{kind}: {other:?}"),
        }
        Form::parse(&form_with(&format!("kind={kind} extent=240")))
            .unwrap_or_else(|error| panic!("{kind}: {error}"));
    }
    // Nothing else needs one, and saying it would be saying something untrue.
    let error = Form::parse(&form_with("kind=screen extent=240")).unwrap_err();
    assert!(
        matches!(error.reason, Reason::UnknownFormProperty { .. }),
        "{error}"
    );
}

#[test]
fn a_side_that_is_not_a_side_says_which_ones_are() {
    let error = Form::parse(&form_with("kind=drawer extent=200 side=sideways")).unwrap_err();
    assert!(error.to_string().contains("before"), "{error}");
}

#[test]
fn the_form_reports_the_defaults_the_documentation_promises() {
    let plain = Form::parse(&form_with("")).expect("parses");
    assert!(
        plain.resizable(),
        "a window is resizable unless it says not"
    );
    assert_eq!(plain.min_size(), None);
    assert_eq!(plain.dim(), 160);
    assert_eq!(plain.extent(), 0);

    // The two edge kinds default to different edges: a drawer is a side panel,
    // a shelf is a bar.
    let drawer = Form::parse(&form_with("kind=drawer extent=200")).expect("parses");
    assert_eq!(drawer.side(), denise_ui::Side::Before);
    assert_eq!(drawer.extent(), 200);
    let shelf = Form::parse(&form_with("kind=shelf extent=64")).expect("parses");
    assert_eq!(shelf.side(), denise_ui::Side::Below);

    let window =
        Form::parse(&form_with("kind=window resizable=#false min-width=320")).expect("parses");
    assert!(!window.resizable());
    assert_eq!(window.min_size(), Some(Size::new(320, 0)));

    let dialog = Form::parse(&form_with("kind=dialog dim=0")).expect("parses");
    assert_eq!(dialog.dim(), 0);
    assert_eq!(
        dialog.side(),
        denise_ui::Side::Before,
        "a dialog has no side"
    );
}

#[test]
fn every_form_property_is_described_once_and_only_by_the_kinds_that_have_it() {
    use denise_forms::{FORM_PROPERTIES, FormKind, form_property, kind_properties};

    let kinds = [
        FormKind::Screen,
        FormKind::Window,
        FormKind::Dialog,
        FormKind::Drawer,
        FormKind::Shelf,
        FormKind::Fragment,
    ];
    for kind in kinds {
        for property in kind_properties(kind) {
            assert!(
                !FORM_PROPERTIES.iter().any(|it| it.name == property.name),
                "`{}` is both everybody's and {kind:?}'s",
                property.name
            );
            assert!(form_property(kind, property.name).is_some());
        }
        // Everything every form has, this kind has too.
        for property in FORM_PROPERTIES {
            assert!(form_property(kind, property.name).is_some(), "{kind:?}");
        }
        assert!(
            form_property(kind, "version").is_none(),
            "version is not editable"
        );
    }
    // A window's is not a dialog's.
    assert!(form_property(FormKind::Dialog, "resizable").is_none());
    assert!(form_property(FormKind::Window, "dim").is_none());
}

#[test]
fn a_new_form_writes_only_what_is_not_a_default() {
    use denise_forms::{FormKind, seed_form};

    // A screen at 800x480 is defaults all the way down except its size.
    let screen = seed_form("Untitled", FormKind::Screen, Size::new(800, 480));
    assert_eq!(screen, "form \"Untitled\" version=1 width=800 height=480\n");
    assert!(!screen.contains("kind="), "the default kind was written");
    assert!(!screen.contains("theme="), "the default theme was written");

    // Every kind's seed loads, and says its kind when its kind is not the
    // default one.
    for kind in [
        FormKind::Screen,
        FormKind::Window,
        FormKind::Dialog,
        FormKind::Drawer,
        FormKind::Shelf,
        FormKind::Fragment,
    ] {
        let source = seed_form("New", kind, Size::new(1024, 600));
        let form =
            Form::parse(&source).unwrap_or_else(|error| panic!("{kind:?}: {error}\n{source}"));
        assert_eq!(form.kind(), kind, "{source}");
        assert_eq!(form.title(), "New");
        assert_eq!(form.size(), Size::new(1024, 600));

        // What comes in from an edge must say how far, and a third of the axis
        // it comes in along is something somebody will recognise.
        if matches!(kind, FormKind::Drawer | FormKind::Shelf) {
            let along = match kind.default_side() {
                denise_ui::Side::Above | denise_ui::Side::Below => 600,
                _ => 1024,
            };
            assert_eq!(form.extent(), along / 3, "{kind:?}");
        }
    }
}

#[test]
fn every_kind_says_what_it_is_for() {
    use denise_forms::FormKind;
    let mut seen: Vec<&str> = Vec::new();
    for (index, name) in FormKind::NAMES.iter().enumerate() {
        let kind = [
            FormKind::Screen,
            FormKind::Window,
            FormKind::Dialog,
            FormKind::Drawer,
            FormKind::Shelf,
            FormKind::Fragment,
        ][index];
        let what = kind.what();
        assert!(what.len() > 20, "`{name}` explains nothing");
        assert!(!seen.contains(&what), "two kinds share a line: {what}");
        seen.push(what);
    }
}
