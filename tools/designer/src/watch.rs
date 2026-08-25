//! Noticing that the other editor saved, and saying what it changed.
//!
//! The whole premise of a text format is that a text editor is the other half of
//! it, so both are open on the same file at once and either may write it. This is
//! the half of that the designer owes: a file that changes underneath is read
//! again rather than quietly overwritten later.
//!
//! # Why this polls rather than subscribes
//!
//! The obvious answer is `notify`, and it would buy nothing here. A change has to
//! reach the designer *through its event loop*, and the loop sleeps until a frame
//! is due or input arrives — there is no way to wake it from a watcher thread. So
//! the loop has to ask on a cadence whatever the mechanism, and a dependency on
//! three platforms' file-notification APIs would not shorten that cadence by a
//! millisecond. Asking also works on the network filesystems where those quietly
//! do not.
//!
//! # Why it reads the file rather than stat-ing it
//!
//! This did compare a timestamp and a length first, and read the file only when
//! one of them moved. It was wrong, and Windows CI is what said so: the system
//! clock ticks about every 16 ms, so a write landing in the same tick as the
//! previous one carries the same timestamp, and a change that happens not to
//! alter the file's length is then invisible. A watcher that misses an edit is
//! worse than no watcher, because it is trusted.
//!
//! So the bytes are the whole answer, and the stat bought nothing worth that. A
//! form file is a few kilobytes — the reference form, which is every node kind
//! this toolkit has, is under nine — and reading one measures at **29 µs**,
//! nearly all of it the open and the close rather than the size. Against the
//! [`EVERY`] cadence that is 0.007% of a second; against the 60 Hz the designer
//! only reaches while something is animating, and therefore painting, it is
//! 0.17% of a frame. Comparing the text also means a `touch`, or a save that
//! rewrote a file identically, is correctly not a change at all.

use std::path::Path;
use std::time::Duration;

use denise_forms::{Form, Written};

/// How long the designer may go without looking at the file under it.
///
/// Long enough to cost nothing on an idle machine, and short enough for #100's
/// "the node moves on the canvas within a second". `Designer::next_frame_in`
/// is what turns this into a cadence: an idle designer with a file open wakes
/// this often, and one with something animating in it looks more often than
/// this and pays nothing extra for it.
pub const EVERY: Duration = Duration::from_millis(400);

/// What the file held the last time the designer looked at it or wrote it.
///
/// The point of holding the text is that "somebody else changed this" is then a
/// question about bytes. It is *not* the form in memory: the form has the
/// designer's unsaved edits in it, and those are exactly what must not be
/// mistaken for the other editor's.
#[derive(Clone, Debug, Default)]
pub struct Watch {
    text: String,
}

impl Watch {
    /// Records what a file holds, at the moment it was read or written.
    pub fn seen(text: &str) -> Self {
        Self {
            text: text.to_string(),
        }
    }

    /// The file's text, if it is not the text this last saw.
    ///
    /// Reporting a change is also taking note of it, so one write is one
    /// question: a file somebody has broken is complained about once rather than
    /// twice a second until they fix it, and an answer given to the sheet is not
    /// asked again on the next frame.
    pub fn changed(&mut self, path: &Path) -> Option<String> {
        let text = std::fs::read_to_string(path).ok()?;
        if text == self.text {
            return None;
        }
        self.text.clone_from(&text);
        Some(text)
    }

    /// Takes this text as what the file holds, without reading it as a change.
    ///
    /// What saving does — the designer's own write must never come back as
    /// somebody else's edit — and what adopting the other editor's version does.
    pub fn agree(&mut self, text: &str) {
        self.text.clear();
        self.text.push_str(text);
    }
}

/// What happened to one node between two versions of a form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// It is in the other version and not in this one.
    Added,
    /// It is in this version and not in the other.
    Removed,
    /// It is in both and says something different.
    Changed,
}

impl Change {
    const fn word(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Removed => "removed",
            Self::Changed => "changed",
        }
    }
}

/// One line of "here is what the other editor did".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Difference {
    pub change: Change,
    /// How to say which node this is, to somebody who has the file open.
    pub who: String,
}

impl Difference {
    /// The line as the conflict sheet shows it.
    pub fn line(&self) -> String {
        format!("{} — {}", self.who, self.change.word())
    }
}

/// What the file on disk has that the form in hand does not, and the reverse.
///
/// Named nodes are matched by name and the rest by position, which is the same
/// order of preference a person reading the two files would use: a `name=` is
/// the one piece of identity a form node carries, and it survives being moved.
/// Everything else is only where it is, so a node inserted above an unnamed one
/// does make the unnamed one look changed — honestly so, since nothing in the
/// file says otherwise.
pub fn differences(mine: &Form, theirs: &Form) -> Vec<Difference> {
    let ours = mine.written();
    let others = theirs.written();
    let mut out = Vec::new();
    let mut matched = vec![false; others.len()];

    for node in &ours {
        let found = others
            .iter()
            .position(|other| pairs(node, other))
            .or_else(|| {
                // Only fall back to position for a node with no name of its own;
                // a named node that is not there by name has gone, whatever sits
                // where it used to.
                (node.name.is_none())
                    .then(|| {
                        others
                            .iter()
                            .position(|other| other.name.is_none() && other.path == node.path)
                    })
                    .flatten()
            });
        match found {
            Some(at) => {
                matched[at] = true;
                if others[at].line != node.line {
                    out.push(Difference {
                        change: Change::Changed,
                        who: name_of(node),
                    });
                }
            }
            None => out.push(Difference {
                change: Change::Removed,
                who: name_of(node),
            }),
        }
    }

    for (node, _) in others.iter().zip(&matched).filter(|(_, seen)| !**seen) {
        out.push(Difference {
            change: Change::Added,
            who: name_of(node),
        });
    }
    out
}

/// Whether two written nodes are the same node under two versions.
fn pairs(mine: &Written, theirs: &Written) -> bool {
    match (&mine.name, &theirs.name) {
        (Some(ours), Some(other)) => ours == other,
        _ => false,
    }
}

/// What to call a node in a list a person reads.
///
/// Its name if it has one, because that is what they typed; then what it says,
/// because a `label "Save"` is recognisable on sight; and only then where it
/// sits, because by that point there is nothing else to go on.
fn name_of(node: &Written) -> String {
    if let Some(name) = &node.name {
        return format!("`{name}`");
    }
    if node.path.is_empty() {
        return String::from("the form itself");
    }
    if let Some(argument) = &node.argument {
        return format!("{} {argument:?}", node.kind);
    }
    let at: Vec<String> = node.path.iter().map(usize::to_string).collect();
    format!("{} at {}", node.kind, at.join("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(body: &str) -> Form {
        Form::parse(&format!(
            "form \"F\" version=1 width=99 height=99 {{\n{body}}}\n"
        ))
        .expect("a test form parses")
    }

    #[test]
    fn a_file_nobody_touched_is_no_change_at_all() {
        let mine = form("    label \"a\" name=one x=0 y=0 w=9 h=9\n");
        assert!(differences(&mine, &mine.clone()).is_empty());
    }

    #[test]
    fn spacing_and_comments_are_not_changes_because_nothing_moved() {
        let mine = form("    label \"a\" name=one x=0 y=0 w=9 h=9\n");
        let theirs =
            form("    // a note somebody added\n    label \"a\"  name=one  x=0  y=0  w=9  h=9\n");
        assert_eq!(differences(&mine, &theirs), Vec::new());
    }

    #[test]
    fn a_rectangle_moved_in_a_text_editor_names_the_node_that_moved() {
        let mine = form(
            "    label \"a\" name=one x=0 y=0 w=9 h=9\n    label \"b\" name=two x=0 y=20 w=9 h=9\n",
        );
        let theirs = form(
            "    label \"a\" name=one x=40 y=0 w=9 h=9\n    label \"b\" name=two x=0 y=20 w=9 h=9\n",
        );
        let found = differences(&mine, &theirs);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].line(), "`one` — changed");
    }

    #[test]
    fn a_named_node_that_moved_in_the_file_is_the_same_node() {
        // Swapped round, and both still there: matching by position would call
        // this two changes, which is the reason names are tried first.
        let mine = form(
            "    label \"a\" name=one x=0 y=0 w=9 h=9\n    label \"b\" name=two x=0 y=20 w=9 h=9\n",
        );
        let theirs = form(
            "    label \"b\" name=two x=0 y=20 w=9 h=9\n    label \"a\" name=one x=0 y=0 w=9 h=9\n",
        );
        assert_eq!(differences(&mine, &theirs), Vec::new());
    }

    #[test]
    fn a_node_added_and_a_node_taken_away_are_told_apart() {
        let mine = form("    label \"a\" name=one x=0 y=0 w=9 h=9\n");
        let theirs = form("    button \"Go\" name=go x=0 y=0 w=9 h=9\n");
        let found: Vec<String> = differences(&mine, &theirs)
            .iter()
            .map(Difference::line)
            .collect();
        assert_eq!(found, ["`one` — removed", "`go` — added"]);
    }

    #[test]
    fn the_forms_own_properties_are_one_of_the_nodes() {
        let mine = form("    label \"a\" name=one x=0 y=0 w=9 h=9\n");
        let theirs = Form::parse(
            "form \"F\" version=1 width=800 height=99 {\n    label \"a\" name=one x=0 y=0 w=9 h=9\n}\n",
        )
        .expect("a form");
        let found = differences(&mine, &theirs);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].line(), "the form itself — changed");
    }

    #[test]
    fn an_unnamed_node_is_matched_where_it_sits_and_named_by_what_it_says() {
        let mine = form("    label \"a\" x=0 y=0 w=9 h=9\n");
        let theirs = form("    label \"a\" x=0 y=8 w=9 h=9\n");
        let found = differences(&mine, &theirs);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].line(), "label \"a\" — changed");
    }

    #[test]
    fn a_node_with_nothing_to_call_it_by_is_named_by_where_it_sits() {
        let mine = form("    panel x=0 y=0 w=9 h=9\n");
        let theirs = form("    panel x=0 y=8 w=9 h=9\n");
        let found = differences(&mine, &theirs);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].line(), "panel at 0 — changed");
    }

    #[test]
    fn a_write_that_changed_nothing_is_not_somebody_elses_edit() {
        let path = std::env::temp_dir().join(format!(
            "denise-watch-{}-{}.dform",
            std::process::id(),
            line!()
        ));
        let text = "form \"F\" version=1 width=99 height=99 { }\n";
        std::fs::write(&path, text).expect("a temporary file");
        let mut watch = Watch::seen(text);

        // The same bytes again, which is what a `touch` or an editor that always
        // writes on save leaves behind.
        std::fs::write(&path, text).expect("written again");
        assert_eq!(watch.changed(&path), None);

        // The case a timestamp cannot answer, and the reason this reads the file:
        // written within the same clock tick as the last one, and the same length,
        // so nothing about the file except its contents has moved.
        let theirs = "form \"F\" version=1 width=42 height=99 { }\n";
        assert_eq!(
            theirs.len(),
            text.len(),
            "this test is not testing what it says"
        );
        std::fs::write(&path, theirs).expect("written for real");
        assert_eq!(watch.changed(&path).as_deref(), Some(theirs));
        // And only once: reporting a change is taking note of it.
        assert_eq!(watch.changed(&path), None);

        // A file that has gone is not a change either, and does not panic.
        std::fs::remove_file(&path).expect("removable");
        assert_eq!(watch.changed(&path), None);
    }
}
