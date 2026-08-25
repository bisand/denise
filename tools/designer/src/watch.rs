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
//! the loop has to ask on a cadence whatever the mechanism, and once it is asking,
//! `stat` is the whole of what a subscription would have told it. Two syscalls
//! twice a second is less than a dependency on three platforms' file-notification
//! APIs, and it works on the network filesystems where those quietly do not.
//!
//! What the stat cannot answer is whether the *bytes* changed, so it does not
//! try: a stamp that moved is a reason to read the file, and the text is what
//! decides. That is what keeps a `touch`, or the designer's own save landing on a
//! coarse-grained clock, from putting a conflict up over nothing.

use std::path::Path;
use std::time::SystemTime;

use denise_forms::{Form, Written};

/// What a file looked like, cheaply.
///
/// Length as well as time because a second's granularity is still out there —
/// on a network share, on an old filesystem — and two writes within one tick
/// are usually not the same length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
}

impl Stamp {
    fn of(path: &Path) -> Option<Self> {
        let data = std::fs::metadata(path).ok()?;
        Some(Self {
            modified: data.modified().ok(),
            len: data.len(),
        })
    }
}

/// What the designer last read from a file, or last wrote to it.
///
/// Held so that "somebody else changed this" is a question about bytes rather
/// than about timestamps. The text is the last agreed state, which is *not* the
/// form in memory: the form has the designer's unsaved edits in it, and those
/// are exactly what must not be mistaken for the other editor's.
#[derive(Clone, Debug, Default)]
pub struct Watch {
    stamp: Option<Stamp>,
    text: String,
}

impl Watch {
    /// Records what a file holds, at the moment it was read or written.
    pub fn agreed(path: &Path, text: &str) -> Self {
        Self {
            stamp: Stamp::of(path),
            text: text.to_string(),
        }
    }

    /// The file's text, if somebody other than this designer has written it.
    ///
    /// `None` when the file is where it was left — including when its timestamp
    /// moved but its contents did not, in which case the stamp is quietly caught
    /// up so the read does not happen again.
    pub fn changed(&mut self, path: &Path) -> Option<String> {
        let stamp = Stamp::of(path);
        if stamp == self.stamp {
            return None;
        }
        self.stamp = stamp;
        let text = std::fs::read_to_string(path).ok()?;
        (text != self.text).then_some(text)
    }

    /// Takes what is on disk as the agreed state without reading it as a change.
    ///
    /// What *Keep mine* does: the designer has been told the other editor's
    /// version is not wanted, so the same change must not be raised twice.
    pub fn accept(&mut self, path: &Path, text: String) {
        self.stamp = Stamp::of(path);
        self.text = text;
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
    fn a_stamp_that_moved_without_the_bytes_moving_is_not_somebody_elses_edit() {
        let path = std::env::temp_dir().join(format!(
            "denise-watch-{}-{}.dform",
            std::process::id(),
            line!()
        ));
        let text = "form \"F\" version=1 width=99 height=99 { }\n";
        std::fs::write(&path, text).expect("a temporary file");
        let mut watch = Watch::agreed(&path, text);

        // The same bytes again, which is what a `touch` or an editor that always
        // writes on save leaves behind.
        std::fs::write(&path, text).expect("written again");
        assert_eq!(watch.changed(&path), None);

        std::fs::write(&path, "form \"F\" version=1 width=42 height=99 { }\n")
            .expect("written for real");
        assert!(watch.changed(&path).is_some());
        // And only once: the stamp caught up as the change was reported.
        assert_eq!(watch.changed(&path), None);

        let _ = std::fs::remove_file(&path);
    }
}
