//! Undo and redo, as two stacks of edits that undo each other.
//!
//! There is no snapshot of anything here. `Form::apply` hands back the edit that
//! reverses the one just made, so undo is applying that, and what *it* hands back
//! is the redo. The whole mechanism is two vectors and a marker for where the
//! file was last saved.
//!
//! # Coalescing
//!
//! A drag is already one edit, because the canvas writes to the document once on
//! release rather than on every pointer move. What needs help is a *run* of small
//! edits that a person thinks of as one: holding an arrow key to nudge something
//! across the form should take one undo to put back, not thirty.
//!
//! So consecutive edits to the same property of the same node merge, and anything
//! that changes what is being worked on — a new selection, a drag beginning, an
//! undo — calls [`History::separate`] to end the run. That is the same rule as
//! "typing in a field coalesces until the focus leaves it", which is what the
//! inspector will want.

use denise_forms::{Edit, Error, Form};

/// The undone and the redoable.
#[derive(Debug, Default)]
pub struct History {
    /// Edits that undo what was done, most recent last.
    undo: Vec<Edit>,
    /// Edits that redo what was undone, most recent last.
    redo: Vec<Edit>,
    /// How deep `undo` was when the file was last written, if that state can
    /// still be reached.
    saved_at: Option<usize>,
    /// Whether the top of `undo` may still absorb another edit.
    open: bool,
}

impl History {
    /// A history for a file just opened, which is to say saved.
    pub fn new() -> Self {
        Self {
            saved_at: Some(0),
            ..Self::default()
        }
    }

    /// Records the edit that undoes what was just done.
    ///
    /// Doing something new discards the redo branch, because there is no longer
    /// one thing that "forward" could mean.
    pub fn record(&mut self, inverse: Edit) {
        self.redo.clear();

        // A run of edits to one property of one node is one step, and so is a
        // run of edits to one node's argument — which is what a person typing
        // into an inspector field is doing.
        if self.open
            && self
                .undo
                .last()
                .is_some_and(|top| same_target(top, &inverse))
        {
            // Keep the older inverse: it holds the value from before the run
            // began, which is where undo has to go back to.
            return;
        }

        // A new edit made after undoing past the save point puts the file in a
        // state the save marker can no longer describe.
        if self.saved_at.is_some_and(|at| at > self.undo.len()) {
            self.saved_at = None;
        }
        self.undo.push(inverse);
        self.open = true;
    }

    /// Ends the current run, so the next edit starts its own step.
    pub fn separate(&mut self) {
        self.open = false;
    }

    /// Undoes one step. `Ok(false)` when there is nothing to undo.
    pub fn undo(&mut self, form: &mut Form) -> Result<bool, Error> {
        self.separate();
        let Some(edit) = self.undo.pop() else {
            return Ok(false);
        };
        match form.apply(edit) {
            Ok(inverse) => {
                self.redo.push(inverse);
                Ok(true)
            }
            Err(error) => Err(error),
        }
    }

    /// Redoes one step. `Ok(false)` when there is nothing to redo.
    pub fn redo(&mut self, form: &mut Form) -> Result<bool, Error> {
        self.separate();
        let Some(edit) = self.redo.pop() else {
            return Ok(false);
        };
        match form.apply(edit) {
            Ok(inverse) => {
                self.undo.push(inverse);
                Ok(true)
            }
            Err(error) => Err(error),
        }
    }

    /// Notes that the file has been written.
    pub fn saved(&mut self) {
        self.saved_at = Some(self.undo.len());
        self.separate();
    }

    /// Whether the file on disk is behind what is on screen.
    ///
    /// Undoing back to where the last save was makes it clean again, which is
    /// what somebody who changed their mind expects.
    pub fn is_dirty(&self) -> bool {
        self.saved_at != Some(self.undo.len())
    }

    /// Whether there is anything to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether there is anything to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// How many steps are on each stack, for a status line and for tests.
    pub fn depth(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
}

/// Whether two edits are the same person doing the same thing.
///
/// Only the small, repeatable edits coalesce. A removal or an insertion is
/// always its own step: nobody holds a key down deleting the same node twice.
fn same_target(a: &Edit, b: &Edit) -> bool {
    match (a, b) {
        (
            Edit::Property {
                path: one,
                name: first,
                ..
            },
            Edit::Property {
                path: other,
                name: second,
                ..
            },
        ) => one == other && first == second,
        (Edit::Argument { path: one, .. }, Edit::Argument { path: other, .. }) => one == other,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "form \"H\" version=1 width=99 height=99 {\n    \
                          label \"a\" x=1 y=2 w=3 h=4\n    \
                          label \"b\" x=5 y=6 w=7 h=8\n}\n";

    fn form() -> Form {
        Form::parse(SOURCE).expect("parses")
    }

    /// Makes an edit through the history, as the designer does.
    fn edit(history: &mut History, form: &mut Form, path: &[usize], name: &str, value: i64) {
        let inverse = form
            .apply(Edit::number(path, name, Some(value)))
            .expect("applied");
        history.record(inverse);
    }

    #[test]
    fn undo_puts_the_file_back_and_redo_puts_it_forward_again() {
        let (mut history, mut form) = (History::new(), form());
        edit(&mut history, &mut form, &[0], "x", 40);
        let after = form.text();
        assert_ne!(after, SOURCE);

        assert!(history.undo(&mut form).expect("undo"));
        assert_eq!(form.text(), SOURCE);
        assert!(history.redo(&mut form).expect("redo"));
        assert_eq!(form.text(), after, "redo did not put back what undo took");
    }

    #[test]
    fn undoing_everything_and_redoing_everything_returns_to_the_same_place() {
        let (mut history, mut form) = (History::new(), form());
        for (index, value) in [(0usize, 11), (1, 22), (0, 33)] {
            history.separate();
            edit(&mut history, &mut form, &[index], "x", value);
        }
        let done = form.text();

        while history.undo(&mut form).expect("undo") {}
        assert_eq!(form.text(), SOURCE);
        while history.redo(&mut form).expect("redo") {}
        assert_eq!(form.text(), done);
    }

    #[test]
    fn a_new_edit_after_an_undo_discards_the_redo_branch() {
        let (mut history, mut form) = (History::new(), form());
        edit(&mut history, &mut form, &[0], "x", 40);
        history.undo(&mut form).expect("undo");
        assert!(history.can_redo());

        history.separate();
        edit(&mut history, &mut form, &[1], "y", 50);
        assert!(!history.can_redo(), "the redo branch survived a new edit");
        assert!(!history.redo(&mut form).expect("nothing to redo"));
    }

    #[test]
    fn a_run_of_edits_to_one_property_is_one_step() {
        let (mut history, mut form) = (History::new(), form());
        for value in 2..=20 {
            edit(&mut history, &mut form, &[0], "x", value);
        }
        assert_eq!(
            history.depth().0,
            1,
            "a run of nudges was {:?} steps",
            history.depth()
        );

        history.undo(&mut form).expect("undo");
        assert_eq!(
            form.text(),
            SOURCE,
            "one undo did not put the whole run back"
        );
    }

    #[test]
    fn a_separated_run_is_two_steps() {
        let (mut history, mut form) = (History::new(), form());
        edit(&mut history, &mut form, &[0], "x", 10);
        history.separate();
        edit(&mut history, &mut form, &[0], "x", 20);
        assert_eq!(history.depth().0, 2);

        history.undo(&mut form).expect("undo");
        assert!(form.text().contains("x=10"), "{}", form.text());
    }

    #[test]
    fn a_different_property_is_never_absorbed_into_a_run() {
        let (mut history, mut form) = (History::new(), form());
        edit(&mut history, &mut form, &[0], "x", 10);
        edit(&mut history, &mut form, &[0], "y", 10);
        edit(&mut history, &mut form, &[1], "x", 10);
        assert_eq!(history.depth().0, 3);
    }

    #[test]
    fn saving_makes_it_clean_and_undoing_back_to_the_save_makes_it_clean_again() {
        let (mut history, mut form) = (History::new(), form());
        assert!(!history.is_dirty(), "a file just opened is not modified");

        edit(&mut history, &mut form, &[0], "x", 40);
        assert!(history.is_dirty());

        history.saved();
        assert!(!history.is_dirty());

        history.separate();
        edit(&mut history, &mut form, &[1], "x", 40);
        assert!(history.is_dirty());
        history.undo(&mut form).expect("undo");
        assert!(
            !history.is_dirty(),
            "undoing back to the save left it dirty"
        );
    }

    #[test]
    fn a_state_the_save_marker_can_no_longer_describe_stays_dirty() {
        // Save, undo past the save, then do something else. The stack is one deep
        // again — the same depth it was when saved — and the file is *not* what
        // was saved, so the marker has to give up rather than lie.
        let (mut history, mut form) = (History::new(), form());
        edit(&mut history, &mut form, &[0], "x", 40);
        history.saved();
        assert!(!history.is_dirty());

        history.undo(&mut form).expect("undo");
        assert!(history.is_dirty());

        history.separate();
        edit(&mut history, &mut form, &[1], "y", 90);
        assert_eq!(history.depth().0, 1, "the same depth as when it was saved");
        assert!(
            history.is_dirty(),
            "it claimed to be the file that was saved"
        );
    }

    #[test]
    fn there_is_nothing_to_undo_at_the_start_and_nothing_breaks_asking() {
        let (mut history, mut form) = (History::new(), form());
        assert!(!history.can_undo() && !history.can_redo());
        assert!(!history.undo(&mut form).expect("nothing"));
        assert!(!history.redo(&mut form).expect("nothing"));
        assert_eq!(form.text(), SOURCE);
    }
}
