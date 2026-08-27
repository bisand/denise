//! Every edit is reversible, and reversing them all is byte-for-byte.
//!
//! This is the property the whole file format is built to make possible, and the
//! one undo depends on: an edit knows its own inverse, so a stack of inverses put
//! back in order restores the document exactly — comments, blank lines, column
//! alignment and all. Nothing here snapshots anything.

use denise_forms::{Edit, Form, Literal, Reason, fragment};

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

// --------------------------------------------------------------- insertion

#[test]
fn a_dropped_node_lands_indented_like_the_ones_around_it() {
    let mut form = Form::parse(SOURCE).expect("parses");
    let undo = form
        .apply(Edit::Insert {
            parent: Vec::new(),
            index: 4,
            text: denise_forms::seed("button", denise::Rect::new(8, 8, 100, 32)),
        })
        .expect("dropped");

    let after = form.text();
    assert!(
        after.contains("\n    button \"button\" x=8 y=8 w=100 h=32\n}"),
        "it did not land on its own indented line:\n{after}"
    );
    Form::parse(&after).expect("still a form");

    form.apply(undo).expect("undone");
    assert_eq!(form.text(), SOURCE, "undoing a drop was not exact");
}

#[test]
fn a_drop_into_a_panel_that_has_no_braces_yet_makes_them_and_undoes_them_away() {
    // The case a designer hits the moment somebody places a panel and then
    // places something in it. Undoing has to take the braces back too, or the
    // file keeps an empty pair nobody typed.
    let source = "form \"F\" version=1 width=99 height=99 {\n    \
                  panel name=card x=1 y=2 w=90 h=80\n}\n";
    let mut form = Form::parse(source).expect("parses");

    let undo = form
        .apply(Edit::Insert {
            parent: vec![0],
            index: 0,
            text: denise_forms::seed("label", denise::Rect::new(4, 4, 120, 20)),
        })
        .expect("dropped into the panel");

    let after = form.text();
    assert!(
        after.contains("panel name=card x=1 y=2 w=90 h=80 {\n        label"),
        "the block is not where it should be:\n{after}"
    );
    assert!(
        after.contains("\n    }\n}"),
        "the brace did not line up:\n{after}"
    );
    let back = Form::parse(&after).expect("still a form");
    assert_eq!(
        back.text(),
        after,
        "what it wrote does not read back the same"
    );

    form.apply(undo).expect("undone");
    assert_eq!(form.text(), source, "the braces outlived the node");
}

#[test]
fn dropping_the_first_node_on_a_form_with_no_children_at_all_works() {
    let source = "form \"F\" version=1 width=99 height=99\n";
    let mut form = Form::parse(source).expect("parses");

    let undo = form
        .apply(Edit::Insert {
            parent: Vec::new(),
            index: 0,
            text: denise_forms::seed("panel", denise::Rect::new(0, 0, 40, 40)),
        })
        .expect("dropped on an empty form");
    let after = form.text();
    assert!(
        after.contains("{\n    panel x=0 y=0 w=40 h=40\n}"),
        "{after}"
    );
    Form::parse(&after).expect("still a form");

    form.apply(undo).expect("undone");
    assert_eq!(form.text(), source);
}

// -------------------------------------------------------------------- moving

/// A form with two panels, so a node has somewhere to go.
const NESTED: &str = "\
form \"Moves\" version=1 width=400 height=300 {
    label \"one\" x=0 y=0 w=10 h=10
    panel name=left x=0 y=20 w=100 h=100 {
        label \"two\" x=1 y=1 w=10 h=10
        label \"three\" x=1 y=20 w=10 h=10
    }
    panel name=right x=120 y=20 w=100 h=100 {
        label \"four\" x=1 y=1 w=10 h=10
    }
    label \"five\" x=0 y=140 w=10 h=10
}
";

#[test]
fn reordering_among_siblings_undoes_exactly() {
    let mut form = Form::parse(NESTED).expect("parses");
    // The last label to the front.
    let undo = form.apply(Edit::move_to(&[3], &[], 0)).expect("moved");

    let after = form.text();
    let order: Vec<&str> = after
        .lines()
        .filter(|line| {
            line.trim_start().starts_with("label") || line.trim_start().starts_with("panel")
        })
        .collect();
    assert!(order[0].contains("\"five\""), "{after}");
    assert!(order[1].contains("\"one\""), "{after}");
    Form::parse(&after).expect("still a form");

    form.apply(undo).expect("undone");
    assert_eq!(form.text(), NESTED);
}

// ------------------------------------------------------- copying and pasting

#[test]
fn the_source_of_a_node_comes_back_without_the_indentation_it_stood_in() {
    let form = Form::parse(NESTED).expect("parses");
    assert_eq!(
        form.node_text(&[0]).as_deref(),
        Some("label \"one\" x=0 y=0 w=10 h=10\n")
    );
    // A panel brings its children, at the depth they have relative to it.
    assert_eq!(
        form.node_text(&[1]).as_deref(),
        Some(concat!(
            "panel name=left x=0 y=20 w=100 h=100 {\n",
            "    label \"two\" x=1 y=1 w=10 h=10\n",
            "    label \"three\" x=1 y=20 w=10 h=10\n",
            "}\n",
        ))
    );
    assert_eq!(form.node_text(&[9]), None);
}

#[test]
fn a_copied_node_pastes_back_in_and_is_laid_out_where_it_lands() {
    let mut form = Form::parse(NESTED).expect("parses");
    let copied = form.node_text(&[1]).expect("the left panel");

    let mut taken: Vec<String> = vec![String::from("left"), String::from("right")];
    let nodes = fragment(&copied, &mut taken).expect("a fragment");
    assert_eq!(nodes.len(), 1);
    assert!(nodes[0].contains("name=left2"), "{}", nodes[0]);

    // Into the *other* panel, which is one level deeper than it came from.
    form.apply(Edit::Insert {
        parent: vec![2],
        index: 1,
        text: nodes[0].clone(),
    })
    .expect("pasted");

    let after = form.text();
    assert!(
        after.contains(concat!(
            "        panel name=left2 x=0 y=20 w=100 h=100 {\n",
            "            label \"two\" x=1 y=1 w=10 h=10\n",
            "            label \"three\" x=1 y=20 w=10 h=10\n",
            "        }\n",
        )),
        "the children did not follow the panel down a level:\n{after}"
    );
    Form::parse(&after).expect("still a form");
}

#[test]
fn undoing_a_paste_takes_the_whole_subtree_back_out() {
    let mut form = Form::parse(NESTED).expect("parses");
    let copied = form.node_text(&[1]).expect("the left panel");
    let mut taken = vec![String::from("left")];
    let nodes = fragment(&copied, &mut taken).expect("a fragment");

    let undo = form
        .apply(Edit::Insert {
            parent: Vec::new(),
            index: 4,
            text: nodes[0].clone(),
        })
        .expect("pasted");
    assert_ne!(form.text(), NESTED);
    form.apply(undo).expect("undone");
    assert_eq!(form.text(), NESTED, "undoing the paste was not exact");
}

#[test]
fn every_name_in_a_pasted_subtree_is_made_unique_and_the_rest_is_left_alone() {
    let mut taken = vec![
        String::from("card"),
        String::from("card2"),
        String::from("title"),
    ];
    let nodes = fragment(
        concat!(
            "panel name=card x=0 y=0 w=10 h=10 {\n",
            "    label \"T\" name=title x=1 y=1 w=2 h=2\n",
            "    label \"U\" name=other x=1 y=4 w=2 h=2\n",
            "}\n",
        ),
        &mut taken,
    )
    .expect("a fragment");

    let text = &nodes[0];
    assert!(text.contains("name=card3"), "{text}");
    assert!(text.contains("name=title2"), "{text}");
    assert!(
        text.contains("name=other"),
        "a free name was changed: {text}"
    );
    // And the caller's list now holds what was settled on, so a second paste
    // does not land on the same names.
    assert!(taken.contains(&String::from("card3")));
    assert!(taken.contains(&String::from("other")));
}

#[test]
fn a_name_that_already_ends_in_a_number_carries_on_from_its_stem() {
    let mut taken = vec![String::from("nav2")];
    let nodes = fragment("label \"N\" name=nav2 x=0 y=0 w=1 h=1", &mut taken).expect("a fragment");
    assert!(nodes[0].contains("name=nav3"), "{}", nodes[0]);
}

#[test]
fn several_nodes_paste_as_several_nodes() {
    let mut taken = Vec::new();
    let nodes = fragment(
        "label \"a\" x=0 y=0 w=1 h=1\nlabel \"b\" x=0 y=4 w=1 h=1\n",
        &mut taken,
    )
    .expect("a fragment");
    assert_eq!(nodes.len(), 2);
    assert!(nodes[0].contains("\"a\""));
    assert!(nodes[1].contains("\"b\""));
}

#[test]
fn nonsense_on_the_clipboard_is_reported_rather_than_pasted() {
    let mut taken = Vec::new();
    let error = fragment(
        "label \"a\" x=0 y=0 w=1 h=1\n{{{ this is not a form",
        &mut taken,
    )
    .expect_err("refused");
    assert!(matches!(error.reason, Reason::Syntax(_)), "{error}");
    assert_eq!(error.at.line, 2, "the line is the fragment's own: {error}");
    assert!(taken.is_empty(), "a refused paste took names anyway");
}

#[test]
fn a_fragment_deeper_than_the_limit_is_refused_rather_than_overflowing() {
    let mut taken = Vec::new();
    let deep = "panel x=0 y=0 w=1 h=1 { ".repeat(denise_forms::MAX_DEPTH + 4);
    assert!(fragment(&deep, &mut taken).is_err());
}

#[test]
fn a_node_moved_to_the_front_and_back_again_leaves_no_blank_line() {
    // Only the first node in a block carries the newline that follows the
    // opening brace. A node arriving in front of it takes that job over, and the
    // one it displaced has to give it up — or the file grows a blank line every
    // time something is brought to the front.
    let mut form = Form::parse(NESTED).expect("parses");
    form.apply(Edit::move_to(&[3], &[], 0))
        .expect("to the front");
    let after = form.text();
    assert!(!after.contains("\n\n"), "a blank line appeared: {after:?}");
    Form::parse(&after).expect("still a form");

    // And there and back is where it started, without anybody undoing.
    form.apply(Edit::move_to(&[0], &[], 3)).expect("and back");
    assert_eq!(form.text(), NESTED);
}

/// A form written with blank lines keeps them when something is re-sequenced.
///
/// The other half of the test above, and #151: a node's leading trivia is *part
/// of it*, which is what carries the comment along — and the blank line was part
/// of the same trivia and was being dropped. The designer's bring-to-front and
/// #98's tab-order mode are both `Edit::Move` among siblings, so somebody who
/// grouped their form with blank lines watched them disappear one at a time.
#[test]
fn a_move_keeps_the_blank_lines_that_travel_with_the_node() {
    // Three groups, separated the way a person separates them.
    const SPACED: &str = "\
form \"Spaced\" version=1 width=400 height=300 {
    label \"one\" x=0 y=0 w=10 h=10

    // why `two` is here
    label \"two\" x=0 y=20 w=10 h=10

    label \"three\" x=0 y=40 w=10 h=10
}
";
    let lines = |text: &str| text.lines().count();

    // Every position a node can be moved to, and back again.
    for (from, to) in [(1usize, 0usize), (2, 1), (0, 2), (2, 0)] {
        let mut form = Form::parse(SPACED).expect("parses");
        form.apply(Edit::move_to(&[from], &[], to)).expect("moved");
        let after = form.text();

        assert_eq!(
            lines(&after),
            lines(SPACED),
            "moving [{from}] to {to} changed the line count:\n{after}"
        );
        assert_eq!(
            after.matches("\n\n").count(),
            SPACED.matches("\n\n").count(),
            "moving [{from}] to {to} lost or invented a blank line:\n{after}"
        );
        // The comment still travels with the node it explains.
        let two = after
            .find("label \"two\"")
            .expect("`two` is still in the file");
        let comment = after.find("// why `two` is here").expect("the comment too");
        assert!(comment < two, "the comment left its node behind:\n{after}");

        // And what it wrote reads back the same, so the blank lines are really
        // in the trivia rather than only in the string.
        let reread = Form::parse(&after).expect("still a form");
        assert_eq!(reread.text(), after);
    }
}

/// There and back is where it started, with no undo, for a form that has them.
///
/// `a_node_moved_to_the_front_and_back_again_leaves_no_blank_line` makes this
/// claim for a fixture with no blank lines in it, where stripping them is
/// indistinguishable from keeping them. This is the same claim where it bites.
#[test]
fn a_move_and_a_move_back_is_where_it_started_even_with_blank_lines() {
    const SPACED: &str = "\
form \"Spaced\" version=1 width=400 height=300 {
    label \"one\" x=0 y=0 w=10 h=10

    label \"two\" x=0 y=20 w=10 h=10

    label \"three\" x=0 y=40 w=10 h=10
}
";
    for (from, to) in [(0usize, 2usize), (2, 0), (1, 2), (2, 1)] {
        let mut form = Form::parse(SPACED).expect("parses");
        form.apply(Edit::move_to(&[from], &[], to)).expect("there");
        form.apply(Edit::move_to(&[to], &[], from)).expect("back");
        assert_eq!(
            form.text(),
            SPACED,
            "[{from}] to {to} and back is not where it started"
        );
    }
}

#[test]
fn a_node_that_changes_depth_is_reindented_and_undone_back() {
    let mut form = Form::parse(NESTED).expect("parses");
    // The top-level `one` into the left panel.
    let undo = form.apply(Edit::move_to(&[0], &[1], 2)).expect("moved");

    let after = form.text();
    assert!(
        after.contains("        label \"three\" x=1 y=20 w=10 h=10\n        label \"one\""),
        "it did not land indented in the panel:\n{after}"
    );
    let back = Form::parse(&after).expect("still a form");
    assert_eq!(
        back.text(),
        after,
        "what it wrote does not read back the same"
    );
    // And the panel it left has one fewer.
    assert_eq!(
        after.matches("label").count(),
        NESTED.matches("label").count()
    );

    form.apply(undo).expect("undone");
    assert_eq!(form.text(), NESTED, "moving it back was not exact");
}

#[test]
fn a_node_moved_out_of_a_panel_loses_the_indentation_it_had_there() {
    let mut form = Form::parse(NESTED).expect("parses");
    let undo = form.apply(Edit::move_to(&[1, 0], &[], 0)).expect("moved");

    let after = form.text();
    assert!(
        after.contains("{\n    label \"two\" x=1 y=1 w=10 h=10\n"),
        "it kept the panel's indentation:\n{after}"
    );
    Form::parse(&after).expect("still a form");

    form.apply(undo).expect("undone");
    assert_eq!(form.text(), NESTED);
}

#[test]
fn a_panel_moved_into_another_takes_its_children_with_it() {
    let mut form = Form::parse(NESTED).expect("parses");
    // `left`, with both its labels, into `right`.
    let undo = form.apply(Edit::move_to(&[1], &[2], 1)).expect("moved");

    let after = form.text();
    assert!(after.contains("        panel name=left"), "{after}");
    assert!(
        after.contains("            label \"two\""),
        "a child did not follow its parent's depth:\n{after}"
    );
    let built = Form::parse(&after).expect("still a form");
    assert_eq!(built.text(), after);

    form.apply(undo).expect("undone");
    assert_eq!(form.text(), NESTED);
}

#[test]
fn a_destination_after_the_source_is_still_the_right_destination() {
    // Taking node [1] out moves [2] to [1]. A move that named [2] as its
    // destination has to end up in the panel it meant, not the one that slid
    // into its place.
    let mut form = Form::parse(NESTED).expect("parses");
    let undo = form.apply(Edit::move_to(&[1], &[2], 0)).expect("moved");

    let after = form.text();
    let right = after
        .find("panel name=right")
        .expect("`right` is still there");
    let left = after
        .find("panel name=left")
        .expect("`left` is still there");
    assert!(left > right, "`left` did not go inside `right`:\n{after}");

    form.apply(undo).expect("undone");
    assert_eq!(form.text(), NESTED);
}

#[test]
fn a_node_cannot_be_moved_inside_itself() {
    let mut form = Form::parse(NESTED).expect("parses");
    let before = form.text();
    for (from, to) in [(vec![1], vec![1]), (vec![1], vec![1, 0]), (vec![], vec![1])] {
        let error = form
            .apply(Edit::Move {
                from: from.clone(),
                to: to.clone(),
                index: 0,
            })
            .expect_err("a tree cannot contain its own root");
        assert!(error.to_string().contains("inside itself"), "{error}");
    }
    assert_eq!(form.text(), before, "a refused move changed the file");
}

#[test]
fn a_node_moved_into_a_panel_that_has_no_braces_makes_them_and_undoes_them_away() {
    let source = "form \"F\" version=1 width=99 height=99 {\n    \
                  label \"a\" x=1 y=2 w=3 h=4\n    \
                  panel name=empty x=0 y=0 w=50 h=50\n}\n";
    let mut form = Form::parse(source).expect("parses");
    let undo = form.apply(Edit::move_to(&[0], &[1], 0)).expect("moved");

    let after = form.text();
    assert!(
        after.contains("panel name=empty x=0 y=0 w=50 h=50 {\n        label \"a\""),
        "{after}"
    );
    Form::parse(&after).expect("still a form");

    form.apply(undo).expect("undone");
    assert_eq!(form.text(), source, "the braces outlived the move");
}

#[test]
fn any_sequence_of_moves_undone_in_order_restores_the_file_exactly() {
    let mut total = 0;
    for seed in 1..=40u64 {
        let mut form = Form::parse(NESTED).expect("parses");
        let mut rolls = Rolls(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut undo: Vec<Edit> = Vec::new();
        let mut applied = 0;

        for _ in 0..12 {
            let depth = 1 + rolls.upto(2);
            let from: Vec<usize> = (0..depth).map(|_| rolls.upto(4)).collect();
            let to: Vec<usize> = (0..rolls.upto(3)).map(|_| rolls.upto(4)).collect();
            let before = form.text();
            match form.apply(Edit::move_to(&from, &to, rolls.upto(4))) {
                Ok(inverse) => {
                    undo.push(inverse);
                    applied += 1;
                    // Every move leaves something that still loads, which is the
                    // other half of reversible.
                    Form::parse(&form.text()).unwrap_or_else(|e| {
                        panic!("seed {seed}: {from:?} -> {to:?} made something unloadable: {e}")
                    });
                }
                Err(_) => assert_eq!(
                    form.text(),
                    before,
                    "seed {seed}: a refused move changed the file"
                ),
            }
        }
        total += applied;

        while let Some(inverse) = undo.pop() {
            form.apply(inverse)
                .unwrap_or_else(|e| panic!("seed {seed}: an inverse would not apply: {e}"));
        }
        assert_eq!(
            form.text(),
            NESTED,
            "seed {seed}: {applied} moves undone did not restore the file"
        );
    }
    assert!(total > 100, "the sweep only applied {total} moves");
}

// ------------------------------------------------------- the form node itself

#[test]
fn the_forms_own_properties_are_edited_by_the_same_door_as_everything_else() {
    let mut form = Form::parse(NESTED).expect("parses");
    assert_eq!(form.size(), denise::Size::new(400, 300));

    // The empty path is the form node.
    let undo = form
        .apply(Edit::number(&[], "width", Some(1024)))
        .expect("resized");
    assert_eq!(form.size(), denise::Size::new(1024, 300));
    assert!(form.text().contains("width=1024"), "{}", form.text());

    form.apply(undo).expect("undone");
    assert_eq!(form.text(), NESTED, "undoing the resize was not exact");
}

#[test]
fn a_form_property_that_was_not_there_is_undone_by_taking_it_away_again() {
    let mut form = Form::parse(NESTED).expect("parses");
    let undo = form
        .apply(Edit::property(
            &[],
            "name",
            Some(Literal::Name(String::from("moves"))),
        ))
        .expect("named");
    assert_eq!(form.name(), Some("moves"));
    form.apply(undo).expect("undone");
    assert_eq!(form.text(), NESTED);
    assert_eq!(form.name(), None);
}

#[test]
fn the_forms_title_is_its_argument_and_edits_like_one() {
    let mut form = Form::parse(NESTED).expect("parses");
    assert_eq!(form.title(), "Moves");
    let undo = form
        .apply(Edit::argument(&[], "Renamed"))
        .expect("retitled");
    assert_eq!(form.title(), "Renamed");
    assert!(form.text().contains("\"Renamed\""), "{}", form.text());
    form.apply(undo).expect("undone");
    assert_eq!(form.text(), NESTED);
}

#[test]
fn a_form_cannot_be_removed_by_naming_the_path_that_addresses_it() {
    let mut form = Form::parse(NESTED).expect("parses");
    assert!(form.apply(Edit::remove(&[])).is_err());
    assert_eq!(form.text(), NESTED);
}
