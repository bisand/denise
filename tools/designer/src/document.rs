//! The form being edited, and its file.

use std::path::{Path, PathBuf};

use denise_forms::Form;

/// A form file, open.
///
/// Holds the **source** as well as the parsed form, and saves the source. That
/// looks redundant and is the whole design: `Form` keeps the `KdlDocument` with
/// its comments, spacing and entry order intact, and every edit the canvas ever
/// makes will be a targeted edit on that document rather than a re-render of a
/// struct. Until there are edits, saving is byte-for-byte what was opened —
/// which is not a placeholder for the round trip, it *is* the round trip, and
/// the test that opens the reference form and saves it asserts exactly that.
pub struct Document {
    path: Option<PathBuf>,
    form: Form,
    dirty: bool,
}

impl Document {
    /// A new, empty form of the given size.
    ///
    /// The kinds and the rest of the form's properties are #90's; this is the one
    /// a `New` button makes until then.
    pub fn blank() -> Self {
        let source = "\
// A new form.
form \"Untitled\" version=1 kind=screen width=800 height=480 theme=dark {
    label \"Hello\" x=24 y=24 w=200 h=24 size=20
}
"
        .to_string();
        let form = Form::parse(&source).expect("the blank form is valid");
        Self {
            path: None,
            form,
            dirty: false,
        }
    }

    /// Opens a file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let source =
            std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let form = Form::parse(&source).map_err(|e| format!("{}:{e}", path.display()))?;
        Ok(Self {
            path: Some(path.to_path_buf()),
            form,
            dirty: false,
        })
    }

    /// Writes the form back, to `to` or to where it came from.
    ///
    /// Through a temporary file and a rename, so a text editor watching this one
    /// never reads a half-written form — the courtesy #100 asks for, and cheaper
    /// to do now than to retrofit.
    pub fn save(&mut self, to: Option<PathBuf>) -> Result<(), String> {
        if let Some(path) = to {
            self.path = Some(path);
        }
        let Some(path) = self.path.clone() else {
            return Err(String::from("this form has never been saved; use Save As"));
        };

        let temporary = path.with_extension("dform.tmp");
        std::fs::write(&temporary, self.form.text())
            .map_err(|e| format!("{}: {e}", temporary.display()))?;
        std::fs::rename(&temporary, &path).map_err(|e| {
            let _ = std::fs::remove_file(&temporary);
            format!("{}: {e}", path.display())
        })?;
        self.dirty = false;
        Ok(())
    }

    /// The parsed form.
    pub fn form(&self) -> &Form {
        &self.form
    }

    /// The form, to edit.
    ///
    /// Every edit goes through here rather than through the text, because the
    /// text is a rendering of the document and the document is what is held.
    pub fn form_mut(&mut self) -> &mut Form {
        &mut self.form
    }

    /// Notes that the file on disk is now behind.
    pub fn touch(&mut self) {
        self.dirty = true;
    }

    /// Re-reads the form from its own edited text.
    ///
    /// Needed when an edit changes the *shape* of the tree rather than a number
    /// in it — a node removed, say — since the paths of everything after it move.
    /// A rectangle changing needs none of this.
    pub fn reparse(&mut self) -> Result<(), String> {
        let text = self.form.text();
        self.form = Form::parse(&text).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// The file this came from, if it came from one.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The directory a picture's `src=` is relative to.
    pub fn base(&self) -> PathBuf {
        self.path()
            .and_then(Path::parent)
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    }

    /// What to put in the title bar: the file's name, or that there is not one.
    pub fn label(&self) -> String {
        let name = self
            .path
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(|| String::from("Untitled"), |n| n.to_string_lossy().into());
        if self.dirty {
            format!("{name} •")
        } else {
            name
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_blank_form_parses_and_is_not_pretending_to_be_saved() {
        let document = Document::blank();
        assert!(document.path().is_none());
        // The bullet is the unsaved marker, and a form nobody has edited has none.
        assert_eq!(document.label(), "Untitled");
        assert_eq!(document.form().size(), denise::Size::new(800, 480));
    }

    #[test]
    fn a_form_that_was_never_saved_cannot_be_saved_without_a_path() {
        let mut document = Document::blank();
        assert!(document.save(None).is_err());
    }
}
