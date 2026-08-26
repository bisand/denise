//! The claim, as a compile failure.
//!
//! `forms/hello.dform` names a node `who`. This asks for `whom`, which is what
//! an application looks like the moment somebody renames that node in the
//! designer — and the point of generating the struct is that it is a compile
//! error here rather than a `None` at runtime.
//!
//! The generated module is written next to this file by `tests/typed.rs` before
//! `trybuild` runs, so it is always the current generator's output rather than a
//! copy that could drift.

include!("hello_form.rs");

fn main() {
    let mut ui: denise_ui::Ui<HelloMessage> =
        denise_ui::Ui::new(denise::Size::new(460, 260), denise::theme::DARK);
    let root = ui.root();
    let form = Hello::build(&mut ui, root).expect("builds");

    // The form calls it `who`.
    let _ = form.whom;
}
