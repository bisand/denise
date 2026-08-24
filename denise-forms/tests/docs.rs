//! Every public function carries an example, and the compiler runs it.
//!
//! This crate's whole job is to be *used* — by an application loading a form, by
//! a designer editing one, by whoever is reading the format documentation with
//! the API open beside it. A signature with prose and no code says what a
//! function is called; an example says what to type.
//!
//! `cargo test --doc` compiles and runs every one of them, so none of them can
//! drift from the API it claims to demonstrate. What *this* test adds is that
//! there is one to run: rustc will warn about a missing doc comment and has no
//! opinion about a missing example, so the opinion lives here.

use std::path::Path;

/// A function whose own doc comment points at another's example is documented by
/// it: `Form::name` does not need its own copy of `Form::title`'s.
const CROSS_REFERENCE: &str = "See [";

#[test]
fn every_public_function_has_an_example_or_points_at_one() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut missing: Vec<String> = Vec::new();
    let mut examined = 0usize;

    let mut sources: Vec<_> = std::fs::read_dir(&root)
        .expect("the source directory is there")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|it| it == "rs"))
        .collect();
    sources.sort();
    assert!(sources.len() >= 4, "found almost no source: {sources:?}");

    for path in sources {
        let text = std::fs::read_to_string(&path).expect("readable");
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            let Some(rest) = trimmed
                .strip_prefix("pub fn ")
                .or_else(|| trimmed.strip_prefix("pub const fn "))
            else {
                continue;
            };
            let name = rest
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .unwrap_or(rest);
            examined += 1;

            // Everything directly above it that is documentation or an
            // attribute, which is the whole of what rustdoc will show.
            let mut doc = String::new();
            let mut above = index;
            while above > 0 {
                let previous = lines[above - 1].trim_start();
                if !previous.starts_with("///") && !previous.starts_with("#[") {
                    break;
                }
                doc.insert_str(0, previous);
                doc.insert(0, '\n');
                above -= 1;
            }

            if !doc.contains("```") && !doc.contains(CROSS_REFERENCE) {
                let file = path.file_name().unwrap_or_default().to_string_lossy();
                missing.push(format!("{file}:{} {name}", index + 1));
            }
        }
    }

    assert!(examined > 30, "this test found almost nothing to check");
    assert!(
        missing.is_empty(),
        "these public functions have no example, and no `{CROSS_REFERENCE}…]` \
         pointing at one:\n  {}",
        missing.join("\n  ")
    );
}
