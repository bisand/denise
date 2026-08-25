//! Forms written by hand, awkwardly, surviving a save that changed nothing.
//!
//! The corpus is [`tests/awkward/`](awkward/README.md) and this walks it — so a
//! new way of writing a form by hand is defended by adding a file, not by adding
//! a test. What it asserts is the promise `docs/forms.md` makes under
//! *Hand-editing* and [#88] is about: files are edited by hand **and** by the
//! designer, alternately, in the same repository, and that only works if opening
//! one and saving it is a no-op on everything nobody touched.
//!
//! This is the format's half — [`Form::parse`] to [`Form::text`]. The designer's
//! half, through a real file on disk, is in `tools/designer/src/app.rs`.
//!
//! [#88]: https://github.com/bisand/denise/issues/88

use std::path::{Path, PathBuf};

use denise_forms::{Edit, Form};

/// Every `.dform` in the corpus, by path, sorted so a failure names the same
/// file every time.
fn corpus() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/awkward");
    let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("the corpus directory is there")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension()? == "dform").then_some(path)
        })
        .collect();
    found.sort();
    assert!(found.len() >= 6, "the corpus went missing: {found:?}");
    found
}

fn name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into()
}

/// One line with every run of whitespace squeezed to a single space.
///
/// What "the same line apart from the number" has to mean: `y=8` becoming
/// `y=16` is a character wider, so the columns somebody lined up shift with it,
/// and that is the edit doing its job rather than reformatting.
fn squeezed(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The lines that differ, which must be the same count on both sides.
fn changed_lines(before: &str, after: &str) -> Vec<String> {
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();
    assert_eq!(old.len(), new.len(), "an edit changed the number of lines");
    old.iter()
        .zip(&new)
        .filter(|(a, b)| a != b)
        .map(|(_, b)| (*b).to_string())
        .collect()
}

#[test]
fn every_awkward_form_in_the_corpus_loads() {
    for path in corpus() {
        let source = std::fs::read_to_string(&path).expect("readable");
        if let Err(error) = Form::parse(&source) {
            panic!("{}: {error}", name(&path));
        }
    }
}

#[test]
fn parsing_one_and_writing_it_back_changes_no_byte() {
    for path in corpus() {
        let source = std::fs::read_to_string(&path).expect("readable");
        let form = Form::parse(&source).expect("parses");
        assert_eq!(
            form.text(),
            source,
            "{} came back different from how it went in",
            name(&path),
        );
    }
}

#[test]
fn moving_one_node_in_any_of_them_is_a_one_line_diff() {
    for path in corpus() {
        let source = std::fs::read_to_string(&path).expect("readable");
        let mut form = Form::parse(&source).expect("parses");

        // The first node under the form, whatever it happens to be, moved eight
        // pixels down — a nudge, the smallest thing a designer does.
        let was: i64 = form
            .property(&[0], "y")
            .expect("every node in the corpus is placed")
            .parse()
            .expect("a whole number");
        let undo = form
            .apply(Edit::number(&[0], "y", Some(was + 8)))
            .unwrap_or_else(|error| panic!("{}: {error}", name(&path)));

        let after = form.text();
        let lines = changed_lines(&source, &after);
        assert_eq!(
            lines.len(),
            1,
            "{} moved one node and changed {} lines: {lines:#?}",
            name(&path),
            lines.len(),
        );
        // Not merely *a* line: the same line, with the number changed and
        // nothing else about it moved. Anything the node was written with —
        // every other property, in the order it was written, and a comment on
        // the end — is still there and still in that order.
        let was_line = source
            .lines()
            .find(|line| line.contains(&format!("y={was}")))
            .expect("the line it was on");
        assert_eq!(
            squeezed(&lines[0]),
            squeezed(was_line).replace(&format!("y={was}"), &format!("y={}", was + 8)),
            "{}: the line came back rewritten rather than edited",
            name(&path),
        );

        // And back again, which is undo: byte for byte, comments and all.
        form.apply(undo).expect("the inverse applies");
        assert_eq!(form.text(), source, "{} did not undo exactly", name(&path));
    }
}

#[test]
fn a_comment_on_the_line_of_a_property_that_then_changed_stays_on_it() {
    // Called out by name in #88, because it is the case a line-rewriting
    // serialiser gets wrong: the comment is trailing trivia on the node, and
    // the property is edited in place beside it.
    let source = "form \"F\" version=1 width=99 height=99 {\n    \
                  label \"hi\" x=10 y=20 w=30 h=40  // eight down from the header\n}\n";
    let mut form = Form::parse(source).expect("parses");
    form.apply(Edit::number(&[0], "y", Some(28)))
        .expect("edits");

    assert!(form.text().contains("y=28"), "{}", form.text());
    assert!(
        form.text().contains("// eight down from the header"),
        "the comment went with the value it was about: {}",
        form.text(),
    );
    assert_eq!(changed_lines(source, &form.text()).len(), 1);
}

#[test]
fn a_property_written_twice_is_still_written_twice_after_a_save() {
    // KDL says the last one wins, and this crate agrees. What it must not do is
    // tidy the file up: somebody edited that line and did not finish, and the
    // fix is theirs to make rather than a tool's to make silently.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/awkward/duplicates.dform");
    let source = std::fs::read_to_string(&path).expect("readable");
    let form = Form::parse(&source).expect("parses");

    assert_eq!(
        form.property(&[0], "x").as_deref(),
        Some("20"),
        "the last one wins"
    );
    assert_eq!(
        form.text().matches("x=").count(),
        source.matches("x=").count()
    );
    assert_eq!(form.text(), source);

    // And the one the file reports is the one an edit replaces, leaving the
    // earlier one exactly where it was — which is what makes this a no-op on
    // everything nobody touched rather than a tidy-up.
    let mut form = form;
    form.apply(Edit::number(&[0], "x", Some(30)))
        .expect("edits");
    let lines = changed_lines(&source, &form.text());
    assert_eq!(lines.len(), 1, "{lines:#?}");
    assert_eq!(
        squeezed(&lines[0]),
        "label \"twice\" x=10 y=10 w=100 h=20 x=30",
        "the duplicate was tidied away rather than left where it was",
    );
}

#[test]
fn a_property_added_is_appended_and_one_reset_to_its_default_is_removed() {
    // The other half of #88's list: order within a node is what the file says,
    // an addition goes on the end, and a reset takes the property out rather
    // than spelling out a default the schema says is not written.
    let source = "form \"F\" version=1 width=99 height=99 {\n    label \"hi\" x=1 y=2 w=3 h=4\n}\n";
    let mut form = Form::parse(source).expect("parses");

    form.apply(Edit::property(
        &[0],
        "role",
        Some(denise_forms::Literal::name("primary")),
    ))
    .expect("adds");
    assert!(
        form.text().contains("x=1 y=2 w=3 h=4 role=primary"),
        "appended out of order: {}",
        form.text(),
    );

    form.apply(Edit::property(&[0], "role", None))
        .expect("removes");
    assert_eq!(
        form.text(),
        source,
        "resetting it did not put the file back"
    );
}
