//! Every edit is reversible, and reversing them all is byte-for-byte.
//!
//! This is the property the whole file format is built to make possible, and the
//! one undo depends on: an edit knows its own inverse, so a stack of inverses put
//! back in order restores the document exactly — comments, blank lines, column
//! alignment and all. Nothing here snapshots anything.

use denise_forms::{Edit, Form, Literal};

fn repo_form(name: &str) -> String {
    let path = format!("{}/../forms/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"))
}

/// A deterministic sequence, so a failure can be repeated from its seed.
///
/// Hand-rolled rather than pulled in: the test needs numbers that are the same
/// every time and different from each other, and that is the whole requirement.
struct Rolls(u64);

impl Rolls {
    fn next(&mut self) -> u64 {
        // xorshift64*, which is short and good enough to shuffle a test.
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn upto(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

const SOURCE: &str = "\
// A form with something of everything worth editing.
form \"Edits\" version=1 kind=screen width=400 height=300 theme=dark {

    // A heading, with a comment above it.
    label \"Heading\"  x=16 y=16  w=200 h=24 size=20

    panel name=card x=16 y=48 w=368 h=200 {
        label \"Name\" x=12 y=12 w=100 h=16   // and one beside it
        text-input name=who x=12 y=32 w=344 h=34 placeholder=\"Ada\"

        panel name=inner x=12 y=76 w=344 h=60 {
            checkbox \"Tick\" name=tick x=8 y=8 w=200 h=24 checked=#true
        }
    }

    button \"Save\" x=16 y=256 w=100 h=28 role=primary on-press=save
}
";

/// One random edit against the document as it now stands.
///
/// Paths are guessed rather than looked up, and a guess that lands nowhere is
/// refused by `apply` — which is itself worth exercising, since a designer with a
/// stale path must get an error and not a corrupted file.
fn roll(rolls: &mut Rolls) -> Edit {
    let depth = 1 + rolls.upto(3);
    let path: Vec<usize> = (0..depth).map(|_| rolls.upto(4)).collect();
    match rolls.upto(16) {
        0..=5 => Edit::number(
            &path,
            ["x", "y", "w", "h", "z"][rolls.upto(5)],
            Some(rolls.upto(500) as i64),
        ),
        6..=7 => Edit::property(
            &path,
            ["z", "w", "role", "placeholder"][rolls.upto(4)],
            None,
        ),
        // The other four shapes a property can take. A guess that lands on a
        // property holding something else is refused, which is the rule about
        // numbers and strings not crossing — and a refusal must leave the
        // document alone just as a missing path does.
        8..=9 => Edit::property(
            &path,
            ["placeholder", "tooltip", "text"][rolls.upto(3)],
            Some(Literal::text(
                ["Ada", "a \"quoted\" one", "line\nbreak"][rolls.upto(3)],
            )),
        ),
        10..=11 => Edit::property(
            &path,
            ["role", "name", "align"][rolls.upto(3)],
            Some(Literal::name(
                ["primary", "secondary", "not an ident"][rolls.upto(3)],
            )),
        ),
        12..=13 => Edit::property(
            &path,
            ["checked", "visible", "enabled"][rolls.upto(3)],
            Some(Literal::Flag(rolls.upto(2) == 0)),
        ),
        14 => Edit::property(
            &path,
            ["size", "value", "x"][rolls.upto(3)],
            Some(Literal::Float(rolls.upto(1000) as f64 / 8.0)),
        ),
        _ => Edit::remove(&path),
    }
}

#[test]
fn any_sequence_of_edits_undone_in_order_restores_the_file_exactly() {
    let mut total = 0;
    for seed in 1..=60u64 {
        let mut form = Form::parse(SOURCE).expect("the fixture parses");
        let mut rolls = Rolls(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut undo: Vec<Edit> = Vec::new();

        let mut applied = 0;
        for _ in 0..25 {
            let edit = roll(&mut rolls);
            // A path that goes nowhere is refused, and refusing must leave the
            // document alone — so nothing is pushed and nothing is owed.
            let before = form.text();
            match form.apply(edit) {
                Ok(inverse) => {
                    undo.push(inverse);
                    applied += 1;
                }
                Err(_) => assert_eq!(
                    form.text(),
                    before,
                    "seed {seed}: a refused edit changed the file"
                ),
            }
        }
        total += applied;

        // Every edit is still a form, which is the other half of reversible.
        Form::parse(&form.text())
            .unwrap_or_else(|e| panic!("seed {seed}: edits made something unloadable: {e}"));

        while let Some(inverse) = undo.pop() {
            form.apply(inverse)
                .unwrap_or_else(|e| panic!("seed {seed}: an inverse would not apply: {e}"));
        }

        assert_eq!(
            form.text(),
            SOURCE,
            "seed {seed}: {applied} edits undone did not restore the file"
        );
    }
    // The guard is over the run rather than each seed: a path is guessed, so one
    // seed may land on nothing much, and what matters is that the whole sweep
    // actually edited things rather than quietly asserting about a file nobody
    // touched.
    assert!(total > 300, "the sweep only applied {total} edits");
}

#[test]
fn undoing_a_removal_brings_back_the_comment_above_it() {
    let mut form = Form::parse(SOURCE).expect("parses");

    // The heading has a comment on the line above, which belongs to it: removing
    // the node takes the comment, and putting it back brings the comment.
    let undo = form.apply(Edit::remove(&[0])).expect("removed");
    let after = form.text();
    assert!(!after.contains("A heading, with a comment"), "{after}");
    assert!(!after.contains("\"Heading\""), "{after}");
    assert!(after.contains("A form with something"), "it took too much");

    form.apply(undo).expect("undone");
    assert_eq!(form.text(), SOURCE);
}

#[test]
fn an_edit_deep_in_a_nest_undoes_as_exactly_as_a_shallow_one() {
    let mut form = Form::parse(SOURCE).expect("parses");
    // form > card > inner > checkbox
    let deep = [1usize, 2, 0];
    let undo = form.apply(Edit::number(&deep, "x", Some(99))).expect("set");
    assert!(form.text().contains("x=99"), "{}", form.text());
    assert!(form.text().contains("name=tick"), "{}", form.text());
    form.apply(undo).expect("undone");
    assert_eq!(form.text(), SOURCE);
}

#[test]
fn a_property_that_was_not_there_is_undone_by_taking_it_away_again() {
    let mut form = Form::parse(SOURCE).expect("parses");
    let undo = form.apply(Edit::number(&[0], "z", Some(4))).expect("set");
    assert!(form.text().contains("z=4"));
    assert_eq!(
        undo,
        Edit::number(&[0], "z", None),
        "the inverse of adding is removing"
    );
    form.apply(undo).expect("undone");
    assert_eq!(form.text(), SOURCE);
}

#[test]
fn a_number_over_a_string_is_refused_rather_than_written() {
    // `placeholder` holds a string, and `placeholder=3` is a file that parses
    // and then will not build. The door refuses it rather than the loader.
    let mut form = Form::parse(SOURCE).expect("parses");
    let before = form.text();
    let error = form
        .apply(Edit::number(&[1, 1], "placeholder", Some(3)))
        .expect_err("a string property is not a number");
    assert!(error.to_string().contains("placeholder"), "{error}");
    assert!(error.to_string().contains("a string"), "{error}");
    assert_eq!(form.text(), before, "a refused edit changed the file");

    // And the other way, on a property holding a number.
    let error = form
        .apply(Edit::property(&[0], "size", Some(Literal::text("large"))))
        .expect_err("a number property is not a string");
    assert!(error.to_string().contains("size"), "{error}");
    assert_eq!(form.text(), before);
}

#[test]
fn a_number_may_gain_a_decimal_point_and_lose_it_again() {
    // Not the rule above: `size=20` becoming `size=20.5` is an ordinary edit,
    // and it is written as a number rather than quoted.
    let mut form = Form::parse(SOURCE).expect("parses");
    let undo = form
        .apply(Edit::property(&[0], "size", Some(Literal::Float(20.5))))
        .expect("a number over a number");
    assert!(form.text().contains("size=20.5"), "{}", form.text());

    form.apply(undo).expect("undone");
    assert_eq!(form.text(), SOURCE);
}

#[test]
fn a_string_is_written_quoted_and_a_name_is_written_bare() {
    let mut form = Form::parse(SOURCE).expect("parses");

    // The difference the two variants exist for: both hold a string, and a
    // form file spells them differently.
    form.apply(Edit::property(
        &[1, 1],
        "placeholder",
        Some(Literal::text("Grace")),
    ))
    .expect("a string");
    assert!(
        form.text().contains("placeholder=\"Grace\""),
        "{}",
        form.text()
    );

    form.apply(Edit::property(
        &[2],
        "role",
        Some(Literal::name("secondary")),
    ))
    .expect("a name");
    assert!(form.text().contains("role=secondary"), "{}", form.text());

    // A name that KDL would not read back bare is quoted anyway, so an edit can
    // never produce a file that stops parsing.
    form.apply(Edit::property(
        &[2],
        "role",
        Some(Literal::name("two words")),
    ))
    .expect("a name needing quotes");
    assert!(
        form.text().contains("role=\"two words\""),
        "{}",
        form.text()
    );
    Form::parse(&form.text()).expect("still a form");
}

#[test]
fn a_string_with_something_awkward_in_it_survives_the_round_trip() {
    let mut form = Form::parse(SOURCE).expect("parses");
    for awkward in [
        "a \"quoted\" word",
        "back\\slash",
        "two\nlines",
        "a\ttab",
        "",
    ] {
        let undo = form
            .apply(Edit::property(
                &[1, 1],
                "placeholder",
                Some(Literal::text(awkward)),
            ))
            .expect("set");
        let text = form.text();
        let back = Form::parse(&text).expect("still a form");
        assert_eq!(
            back.text(),
            text,
            "{awkward:?} did not survive being written"
        );
        form.apply(undo).expect("undone");
        assert_eq!(form.text(), SOURCE, "{awkward:?} did not undo exactly");
    }
}

#[test]
fn a_value_spelled_by_hand_comes_back_spelled_the_same_way() {
    // The reason an inverse carries text rather than a number: `1_000` and
    // `0x10` mean what a plain integer means and are not written the way one
    // would be written. An undo that changed the spelling would be correct and
    // would still have edited a line nobody touched.
    let source = "form \"F\" version=1 width=9 height=9 {\n    \
                  label \"hi\" x=1_000 y=0x10 w=3 h=4 size=20.0\n}\n";
    let mut form = Form::parse(source).expect("parses");

    let mut undo = Vec::new();
    for edit in [
        Edit::number(&[0], "x", Some(5)),
        Edit::number(&[0], "y", Some(6)),
        Edit::property(&[0], "size", Some(Literal::Float(11.5))),
    ] {
        undo.push(form.apply(edit).expect("applied"));
    }
    assert!(form.text().contains("x=5 y=6"), "{}", form.text());

    while let Some(inverse) = undo.pop() {
        form.apply(inverse).expect("undone");
    }
    assert_eq!(form.text(), source);
}

#[test]
fn a_verbatim_that_is_not_one_value_is_refused() {
    let mut form = Form::parse(SOURCE).expect("parses");
    let before = form.text();
    for text in ["", "1 x=2", "x=2", "\"unterminated"] {
        assert!(
            form.apply(Edit::property(
                &[0],
                "size",
                Some(Literal::Verbatim(text.to_string()))
            ))
            .is_err(),
            "accepted {text:?}"
        );
    }
    assert_eq!(form.text(), before);
}

#[test]
fn an_insertion_of_something_that_is_not_one_node_is_refused() {
    let mut form = Form::parse(SOURCE).expect("parses");
    let before = form.text();
    for text in ["", "label \"a\" x=0\nlabel \"b\" x=0", "{{{"] {
        assert!(
            form.apply(Edit::Insert {
                parent: Vec::new(),
                index: 0,
                text: text.to_string(),
            })
            .is_err(),
            "accepted {text:?}"
        );
    }
    assert_eq!(form.text(), before);
}

#[test]
fn the_reference_form_survives_being_edited_and_put_back() {
    let source = repo_form("reference.dform");
    let mut form = Form::parse(&source).expect("parses");
    let mut undo = Vec::new();

    // A move, a resize, a property added, and a whole panel taken out.
    for edit in [
        Edit::number(&[0], "x", Some(4)),
        Edit::number(&[0], "w", Some(999)),
        Edit::number(&[1], "z", Some(7)),
        Edit::remove(&[2]),
    ] {
        undo.push(form.apply(edit).expect("applied"));
    }
    assert_ne!(form.text(), source);
    Form::parse(&form.text()).expect("still a form");

    while let Some(inverse) = undo.pop() {
        form.apply(inverse).expect("undone");
    }
    assert_eq!(form.text(), source);
}

#[test]
fn an_edit_keeps_the_columns_somebody_lined_up_by_hand() {
    // `hello.dform` lines its labels' properties up in columns. An edit that
    // collapsed that would keep the one-line diff and still lose something the
    // author did on purpose — and `kdl`'s obvious call does exactly that.
    let source = repo_form("hello.dform");
    assert!(
        source.contains("\"Hello, Denise\"      x=20"),
        "the fixture stopped being aligned, so this test proves nothing"
    );

    let mut form = Form::parse(&source).expect("parses");
    let undo = form
        .apply(Edit::number(&[0, 0], "y", Some(24)))
        .expect("moved");

    let after = form.text();
    assert!(after.contains("y=24"), "{after}");
    assert!(
        after.contains("\"Hello, Denise\"      x=20"),
        "the columns collapsed: {after}"
    );

    form.apply(undo).expect("undone");
    assert_eq!(form.text(), source);
}

#[test]
fn several_edits_as_one_undo_as_one() {
    let mut form = Form::parse(SOURCE).expect("parses");
    let undo = form
        .apply(Edit::Many(vec![
            Edit::number(&[0], "x", Some(1)),
            Edit::number(&[0], "y", Some(2)),
            Edit::number(&[0], "w", Some(3)),
        ]))
        .expect("applied");

    let after = form.text();
    assert!(after.contains("x=1 y=2  w=3"), "{after}");

    form.apply(undo).expect("undone");
    assert_eq!(form.text(), SOURCE, "one undo did not put all three back");
}

#[test]
fn a_compound_edit_that_cannot_finish_does_not_start() {
    let mut form = Form::parse(SOURCE).expect("parses");
    let before = form.text();

    // The second one names a node that is not there, so the first must not stand.
    let error = form
        .apply(Edit::Many(vec![
            Edit::number(&[0], "x", Some(1)),
            Edit::number(&[99], "x", Some(1)),
        ]))
        .expect_err("half of it was impossible");
    assert!(error.to_string().contains("no node"), "{error}");
    assert_eq!(
        form.text(),
        before,
        "a refused compound left half an edit behind"
    );
}

#[test]
fn a_nodes_argument_is_edited_where_it_stands() {
    // `label "Heading"` carries its text as an argument rather than a property,
    // which is how every form in this repo is written. An inspector editing that
    // text has to change the argument, or the file would say one thing and the
    // screen another.
    let mut form = Form::parse(SOURCE).expect("parses");
    assert_eq!(form.argument(&[0]).as_deref(), Some("Heading"));

    let undo = form
        .apply(Edit::argument(&[0], "A heading"))
        .expect("set the argument");
    assert!(
        form.text().contains(r#"label "A heading""#),
        "{}",
        form.text()
    );
    // And nothing else on the line moved.
    assert!(form.text().contains("x=16 y=16  w=200"), "{}", form.text());

    form.apply(undo).expect("undone");
    assert_eq!(form.text(), SOURCE);
}

#[test]
fn a_node_written_without_an_argument_does_not_grow_one() {
    let mut form = Form::parse(SOURCE).expect("parses");
    let before = form.text();
    // The panel is `panel name=card …`, all properties and no argument.
    let error = form
        .apply(Edit::argument(&[1], "Card"))
        .expect_err("there is nothing there to set");
    assert!(error.to_string().contains("argument"), "{error}");
    assert_eq!(form.text(), before);
    assert_eq!(form.argument(&[1]), None);
}

#[test]
fn what_the_file_writes_is_what_it_reports() {
    let form = Form::parse(SOURCE).expect("parses");

    // A string comes back without its quotes: a field edits the string.
    assert_eq!(
        form.property(&[1, 1], "placeholder").as_deref(),
        Some("Ada")
    );
    assert_eq!(form.property(&[1], "name").as_deref(), Some("card"));
    assert_eq!(form.property(&[0], "x").as_deref(), Some("16"));
    assert_eq!(
        form.property(&[1, 2, 0], "checked").as_deref(),
        Some("#true")
    );

    // Nothing written is the whole of "this is at its default".
    assert_eq!(form.property(&[0], "z"), None);
    assert_eq!(form.property(&[9], "x"), None, "no such node");
}
