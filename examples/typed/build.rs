//! Turns `forms/hello.dform` into a struct and an enum, at build time.
//!
//! The whole of it. `to_out_dir` writes `$OUT_DIR/hello.rs` and tells Cargo to
//! rerun this when the form changes, so editing the form in the designer and
//! pressing build is all it takes.

fn main() {
    let form = concat!(env!("CARGO_MANIFEST_DIR"), "/../../forms/hello.dform");
    if let Err(why) = denise_forms::codegen::to_out_dir(form) {
        // A form that will not generate is a broken build, and the message says
        // which file and which line — the same message `denise-forms check`
        // would have given, at the same moment the compiler would have.
        panic!("{why}");
    }
}
