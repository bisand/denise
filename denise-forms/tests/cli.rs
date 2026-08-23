//! The command-line tool, driven as a command line.
//!
//! Running the real binary rather than calling into it: exit codes and the shape
//! of what it prints are the contract a CI job and a shell script depend on, and
//! neither is visible from inside the crate.

#![cfg(feature = "cli")]

use std::path::PathBuf;
use std::process::{Command, Output};

fn tool() -> Command {
    Command::new(env!("CARGO_BIN_EXE_denise-forms"))
}

fn repo(relative: &str) -> String {
    format!("{}/../{relative}", env!("CARGO_MANIFEST_DIR"))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("denise-forms-tests");
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir.join(name)
}

fn write(name: &str, contents: &str) -> PathBuf {
    let path = scratch(name);
    std::fs::write(&path, contents).expect("writing a fixture");
    path
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

// ---------------------------------------------------------------------- check

#[test]
fn check_accepts_every_form_in_the_repository() {
    let output = tool()
        .arg("check")
        .arg(repo("forms/hello.dform"))
        .arg(repo("forms/reference.dform"))
        .output()
        .expect("running the tool");
    assert!(output.status.success(), "{}", text(&output));
    let said = text(&output);
    assert!(said.contains("Hello"), "{said}");
    assert!(said.contains("1024x600"), "{said}");
    assert!(
        !said.contains("warning"),
        "the repository's own forms should lint clean: {said}"
    );
}

#[test]
fn check_reports_a_broken_form_as_file_line_column_and_exits_one() {
    let path = write(
        "broken.dform",
        "form \"Bad\" version=1 width=200 height=100 {\n    label \"hi\" x=0 y=0 w=50 h=20 colour=red\n}\n",
    );
    let output = tool().arg("check").arg(&path).output().expect("running");
    assert_eq!(output.status.code(), Some(1));

    let said = text(&output);
    let line = said.lines().next().unwrap_or_default();
    // `path:line:col: message`, which is what an editor and a CI annotation both
    // know how to read.
    let tail = line.rsplit(".dform:").next().unwrap_or_default();
    let mut parts = tail.splitn(3, ':');
    assert_eq!(parts.next(), Some("2"), "{said}");
    assert!(
        parts.next().is_some_and(|c| c.parse::<usize>().is_ok()),
        "no column in: {said}"
    );
    assert!(said.contains("colour"), "{said}");
    assert!(said.contains("it accepts"), "{said}");
}

#[test]
fn check_warns_about_geometry_without_failing() {
    let path = write(
        "overlap.dform",
        "form \"O\" version=1 width=200 height=100 {\n\
         \x20   panel name=a x=0 y=0 w=100 h=50\n\
         \x20   panel name=b x=50 y=0 w=100 h=50\n\
         \x20   panel name=c x=0 y=90 w=100 h=100\n\
         }\n",
    );
    let output = tool().arg("check").arg(&path).output().expect("running");
    assert!(
        output.status.success(),
        "a lint is a warning, not a failure"
    );

    let said = text(&output);
    assert!(said.contains("overlaps"), "{said}");
    assert!(said.contains("leaves its parent"), "{said}");

    // And it can be turned off, for a form that means it.
    let quiet = tool()
        .args(["check", "--no-lint"])
        .arg(&path)
        .output()
        .expect("running");
    assert!(quiet.status.success());
    assert!(!text(&quiet).contains("overlaps"), "{}", text(&quiet));
}

#[test]
fn check_says_nothing_when_asked_to_be_quiet() {
    let output = tool()
        .args(["check", "--quiet"])
        .arg(repo("forms/hello.dform"))
        .output()
        .expect("running");
    assert!(output.status.success());
    assert_eq!(text(&output), "", "--quiet should be silent on success");
}

// --------------------------------------------------------------------- render

#[test]
fn render_draws_the_form_at_its_own_size_and_does_it_the_same_way_twice() {
    let once = scratch("ref-1.ppm");
    let twice = scratch("ref-2.ppm");
    for out in [&once, &twice] {
        let output = tool()
            .arg("render")
            .arg(repo("forms/reference.dform"))
            .arg(out)
            .output()
            .expect("running");
        assert!(output.status.success(), "{}", text(&output));
    }

    let first = std::fs::read(&once).expect("the first render");
    let second = std::fs::read(&twice).expect("the second render");

    // A PPM header for exactly the form's declared size.
    assert!(first.starts_with(b"P6\n1024 600\n255\n"), "wrong header");
    assert_eq!(first.len(), "P6\n1024 600\n255\n".len() + 1024 * 600 * 3);
    assert_eq!(
        first, second,
        "two renders of one file must agree, or a snapshot is worthless"
    );

    // Not a blank sheet: a form that drew nothing would still be the right size.
    let body = &first["P6\n1024 600\n255\n".len()..];
    let distinct: std::collections::HashSet<&[u8]> = body
        .as_chunks::<3>()
        .0
        .iter()
        .take(200_000)
        .map(|c| &c[..])
        .collect();
    assert!(distinct.len() > 8, "only {} colours drawn", distinct.len());
}

#[test]
fn render_takes_a_theme_and_the_theme_changes_the_pixels() {
    let dark = scratch("theme-dark.ppm");
    let light = scratch("theme-light.ppm");
    for (out, theme) in [(&dark, "dark"), (&light, "light")] {
        let output = tool()
            .args(["render", &repo("forms/hello.dform")])
            .arg(out)
            .args(["--theme", theme])
            .output()
            .expect("running");
        assert!(output.status.success(), "{}", text(&output));
    }
    assert_ne!(
        std::fs::read(&dark).unwrap(),
        std::fs::read(&light).unwrap(),
        "the light theme drew the dark one"
    );
}

#[test]
fn render_refuses_a_theme_that_is_not_one() {
    let output = tool()
        .args(["render", &repo("forms/hello.dform")])
        .arg(scratch("never.ppm"))
        .args(["--theme", "beige"])
        .output()
        .expect("running");
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output).contains("beige"), "{}", text(&output));
}

// ----------------------------------------------------------------------- shell

#[test]
fn no_command_and_a_wrong_one_both_fail_with_the_usage() {
    for args in [vec![], vec!["frobnicate"]] {
        let output = tool().args(&args).output().expect("running");
        assert_eq!(output.status.code(), Some(1), "for {args:?}");
        assert!(text(&output).contains("denise-forms check"), "for {args:?}");
    }
    let help = tool().arg("--help").output().expect("running");
    assert!(help.status.success());
}
