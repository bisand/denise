//! A form file as Rust the compiler checks.
//!
//! Two kinds of test. The ordinary ones exercise the generator directly — what
//! it makes of an awkward name, what it refuses and why. The `trybuild` ones
//! assert the thing the whole feature exists for and that no ordinary test can
//! say: **code that used the old name does not compile.**

#![cfg(feature = "codegen")]

use denise_forms::Reason;
use denise_forms::codegen::generate;

fn repo(relative: &str) -> String {
    let path = format!("{}/../{relative}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"))
}

fn failure(source: &str) -> Reason {
    match generate(source, "test") {
        Ok(generated) => panic!("expected a failure, got:\n{}", generated.source),
        Err(error) => error.reason,
    }
}

#[test]
fn a_form_becomes_a_struct_of_its_names_and_an_enum_of_its_messages() {
    let generated = generate(&repo("forms/hello.dform"), "hello").expect("generates");

    assert_eq!(generated.kind, "Hello");
    assert_eq!(generated.message, "HelloMessage");

    // The three names `hello.dform` gives, as fields.
    for field in ["pub card:", "pub who:", "pub greeting:"] {
        assert!(
            generated.source.contains(field),
            "no `{field}` in the output"
        );
    }
    // And its one message, once, even though two widgets send it.
    assert_eq!(generated.source.matches("Greet,").count(), 1);
}

#[test]
fn every_payload_shape_becomes_a_variant_that_carries_it() {
    // The reference form uses all four, which is why it is the one to check
    // against: a variant of the wrong shape would not compile where it is used
    // as a `fn(bool) -> M`.
    let generated = generate(&repo("forms/reference.dform"), "reference").expect("generates");
    for variant in [
        "SetNotify(bool)",
        "Navigate(usize)",
        "SetVolume(f32)",
        "Save,",
    ] {
        assert!(
            generated.source.contains(variant),
            "no `{variant}` in the output",
        );
    }
}

#[test]
fn a_kebab_name_becomes_a_snake_field_and_a_pascal_variant() {
    let source = "form \"F\" version=1 width=99 height=99 {\n    \
                  checkbox \"Go\" name=full-name x=0 y=0 w=9 h=9 on-change=set-verbose\n}\n";
    let generated = generate(source, "f").expect("generates");
    assert!(
        generated.source.contains("pub full_name:"),
        "{}",
        generated.source
    );
    assert!(
        generated.source.contains("SetVerbose(bool)"),
        "{}",
        generated.source
    );
}

#[test]
fn a_name_that_is_a_keyword_is_escaped_rather_than_refused() {
    // `type` is a perfectly good name for a field, and `r#type` is exactly what
    // the raw form is for.
    let source = "form \"F\" version=1 width=99 height=99 {\n    \
                  label \"x\" name=type x=0 y=0 w=9 h=9\n}\n";
    let generated = generate(source, "f").expect("generates");
    assert!(
        generated.source.contains("pub r#type:"),
        "{}",
        generated.source
    );
}

#[test]
fn a_name_rust_cannot_spell_at_all_is_refused_with_the_reason() {
    // A form loads perfectly well with a node called `2`. A struct field cannot.
    let source = "form \"F\" version=1 width=99 height=99 {\n    \
                  label \"x\" name=\"2\" x=0 y=0 w=9 h=9\n}\n";
    match failure(source) {
        Reason::NotAnIdentifier { found, .. } => assert_eq!(found, "2"),
        other => panic!("{other:?}"),
    }

    // And the three words Rust will not take even raw.
    let source = "form \"F\" version=1 width=99 height=99 {\n    \
                  label \"x\" name=self x=0 y=0 w=9 h=9\n}\n";
    assert!(matches!(failure(source), Reason::NotAnIdentifier { .. }));
}

#[test]
fn two_names_that_become_one_field_are_refused_by_both_names() {
    // `full-name` and `full_name` are two nodes to a form file and one field to
    // Rust. Generating either silently would drop a node the application asked
    // for.
    let source = "form \"F\" version=1 width=99 height=99 {\n    \
                  label \"a\" name=full-name x=0 y=0 w=9 h=9\n    \
                  label \"b\" name=full_name x=0 y=20 w=9 h=9\n}\n";
    match failure(source) {
        Reason::Collides {
            found,
            with,
            spelled,
        } => {
            assert_eq!(found, "full_name");
            assert_eq!(with, "full-name");
            assert_eq!(spelled, "full_name");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn one_message_name_with_two_shapes_is_refused_rather_than_generated_wrong() {
    // The engine is happy with this: an application's `match` answers each call
    // separately, once as `Plain` and once as `Bool`. A generated enum cannot —
    // `Go` is either a variant or a `fn(bool) -> M`, and not both.
    let source = "form \"F\" version=1 width=99 height=99 {\n    \
                  button \"Go\" x=0 y=0 w=9 h=9 on-press=go\n    \
                  checkbox \"Also\" x=0 y=20 w=9 h=9 on-change=go\n}\n";
    match failure(source) {
        Reason::PayloadClash { found, .. } => assert_eq!(found, "go"),
        other => panic!("{other:?}"),
    }

    // The same name twice with the *same* shape is fine, and is what
    // `hello.dform` does on purpose: pressing Enter and pressing the button are
    // one thing to whoever is using it.
    let source = "form \"F\" version=1 width=99 height=99 {\n    \
                  button \"Go\" x=0 y=0 w=9 h=9 on-press=go\n    \
                  button \"Also\" x=0 y=20 w=9 h=9 on-press=go\n}\n";
    let generated = generate(source, "f").expect("generates");
    assert_eq!(generated.source.matches("Go,").count(), 1);
}

#[test]
fn a_form_that_does_not_parse_fails_generation_with_its_own_position() {
    let generated = generate("form \"F\" version=99 width=1 height=1\n", "f");
    assert!(
        generated.is_err(),
        "a form from the future generated anyway"
    );
}

#[test]
fn a_form_naming_nothing_still_generates_something_that_compiles() {
    let source = "form \"F\" version=1 width=99 height=99 {\n    \
                  label \"x\" x=0 y=0 w=9 h=9\n}\n";
    let generated = generate(source, "f").expect("generates");
    // No fields and no messages, but a struct and an enum all the same — a form
    // gains its first name without the application's `use` line changing.
    assert!(generated.source.contains("pub struct F {"));
    assert!(generated.source.contains("pub enum FMessage {"));
}

/// Writes the current generator's output next to the fixtures, so what
/// `trybuild` compiles is never a stale copy.
fn generate_fixture() {
    let generated = generate(&repo("forms/hello.dform"), "hello").expect("generates");
    let out = format!("{}/tests/ui/hello_form.rs", env!("CARGO_MANIFEST_DIR"));
    std::fs::write(&out, &generated.source).expect("writing the fixture");
}

#[test]
fn using_a_name_the_form_no_longer_has_does_not_compile() {
    // The claim of the whole feature. `hello.dform` names a node `who`; the
    // fixture asks for `whom`, which is what an application looks like the
    // moment somebody renames that node — and it must be a compile error rather
    // than a `None` in front of somebody.
    generate_fixture();
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/renamed.rs");
    cases.compile_fail("tests/ui/added-message.rs");
}
