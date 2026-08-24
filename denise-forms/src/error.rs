//! What went wrong, and where in the file.
//!
//! A form is something a person typed, so every failure here carries a line, a
//! column, and — where there is a finite set of right answers — the whole set. A
//! misspelled property names the property, the widget, and everything that widget
//! *does* accept; there is no error in this crate that leaves somebody grepping.

use std::fmt;

use denise_ui::widgets::Property;

/// Where in the source something is, counted from one as an editor counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct At {
    /// Line, from 1.
    pub line: usize,
    /// Column, from 1.
    pub column: usize,
}

impl At {
    /// The position of a byte offset in `source`.
    ///
    /// Computed on demand rather than carried around: the parser hands back byte
    /// spans, and a form that loads without complaint should not have paid for
    /// counting newlines.
    pub fn of(source: &str, offset: usize) -> Self {
        let offset = offset.min(source.len());
        let consumed = &source[..offset];
        let line = consumed.bytes().filter(|&b| b == b'\n').count() + 1;
        let column = consumed
            .rfind('\n')
            .map_or(offset, |nl| offset - nl - 1)
            // Count characters rather than bytes, so a column in a line with an
            // `æ` in it points where the editor's cursor would be.
            .min(consumed.len());
        let column = consumed[consumed.len() - column..].chars().count() + 1;
        Self { line, column }
    }

    /// The start of the file, for a complaint about the file as a whole.
    pub const START: Self = Self { line: 1, column: 1 };
}

impl fmt::Display for At {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// Everything that can be wrong with a form file.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Reason {
    /// The file is not KDL at all.
    Syntax(String),
    /// The top level is not a single `form` node.
    NotAForm {
        /// What was found there instead.
        found: String,
    },
    /// `version` is missing, or is not a number.
    Version,
    /// The file was written for a later version of this crate.
    FromTheFuture {
        /// What the file asks for.
        wanted: u64,
        /// The highest this crate understands.
        understood: u64,
    },
    /// A required property is not there.
    Missing {
        /// The node's kind.
        kind: String,
        /// What was needed.
        name: &'static str,
    },
    /// No widget goes by that name.
    UnknownWidget {
        /// What the file said.
        found: String,
    },
    /// The widget has no such property.
    UnknownProperty {
        /// The node's kind.
        kind: &'static str,
        /// What the file said.
        found: String,
        /// Everything that widget does accept.
        accepted: &'static [Property],
    },
    /// The property exists; the value was not the shape it takes.
    WrongType {
        /// The node's kind.
        kind: &'static str,
        /// The property.
        name: String,
        /// What it takes, in words.
        wanted: &'static str,
    },
    /// A name that should have been one of a fixed set was not.
    NotAName {
        /// The property, or the thing being named.
        name: String,
        /// What the file said.
        found: String,
        /// Every name that would have worked.
        accepted: &'static [&'static str],
    },
    /// A child node that the parent has no use for.
    UnexpectedChild {
        /// The parent's kind.
        parent: String,
        /// The child's kind.
        found: String,
    },
    /// The application's resolver did not know a message name.
    UnknownMessage {
        /// The name in the file.
        found: String,
    },
    /// The resolver knew the name but gave back the wrong shape of message.
    WrongMessage {
        /// The name in the file.
        found: String,
        /// What the widget needs.
        wanted: &'static str,
    },
    /// A picture the application's loader would not load.
    Asset {
        /// The path in the file.
        path: String,
    },
    /// Two nodes claim the same name.
    DuplicateName {
        /// The name.
        name: String,
    },
    /// More than one node asked for the caret.
    TwoFocuses,
    /// The tree refused a node, which it does when its parent is gone.
    TreeRefused,
    /// The file nests deeper than this crate will follow.
    TooDeep {
        /// The limit.
        limit: usize,
    },
    /// The file is larger than this crate will read.
    TooLarge {
        /// The limit, in bytes.
        limit: usize,
    },
    /// An edit named a node that is not there.
    NoSuchNode {
        /// The child path that went nowhere.
        path: Vec<usize>,
    },
    /// An edit set the positional argument of a node written without one.
    ///
    /// An argument comes before every property, and no edit puts something at
    /// the front of a line without rewriting the line. Setting the property
    /// that argument stands for is what to do instead.
    NoArgument,
    /// An edit would have put a number where a string lives, or the reverse.
    ///
    /// Not a matter of reversibility — an edit's inverse restores the text that
    /// was there, so any value can be put back. It is a matter of what the file
    /// would become: `placeholder=70` parses and then will not build, and an
    /// editor holding the widget's own descriptor already knows better. So the
    /// door refuses it rather than the loader, three steps later.
    ///
    /// A number replacing a number is not this, whichever way it is written:
    /// `value=70` becoming `value=70.5` is an ordinary edit.
    WrongKind {
        /// The property.
        name: String,
        /// What it holds now.
        holds: &'static str,
        /// What the edit offered.
        given: &'static str,
    },
}

/// A failure, and where in the file it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    /// Where.
    pub at: At,
    /// What.
    pub reason: Reason,
}

impl Error {
    pub(crate) const fn new(at: At, reason: Reason) -> Self {
        Self { at, reason }
    }
}

/// Renders a list, Oxford-comma-free and bounded, for an "expected one of" line.
fn listing(names: impl Iterator<Item = impl fmt::Display>) -> String {
    let mut all: Vec<String> = names.map(|n| n.to_string()).collect();
    all.sort_unstable();
    // A widget with thirty properties should not print thirty of them at the top
    // of a diagnostic; the point is to jog a memory, not to be documentation.
    const SHOWN: usize = 12;
    if all.len() > SHOWN {
        let rest = all.len() - SHOWN;
        all.truncate(SHOWN);
        format!("{}, and {rest} more", all.join(", "))
    } else {
        all.join(", ")
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", self.at)?;
        match &self.reason {
            Reason::Syntax(message) => write!(f, "{message}"),
            Reason::NotAForm { found } => write!(
                f,
                "a form file holds one `form` node and nothing else; found `{found}`"
            ),
            Reason::Version => write!(
                f,
                "`form` needs a `version`, as a whole number — this crate reads version {}",
                crate::VERSION
            ),
            Reason::FromTheFuture { wanted, understood } => write!(
                f,
                "this file is version {wanted} and this crate reads version {understood}; \
                 a newer denise-forms will open it"
            ),
            Reason::Missing { kind, name } => {
                write!(f, "`{kind}` needs a `{name}` and there is none")
            }
            Reason::UnknownWidget { found } => {
                let kinds = listing(denise_ui::widgets::all().iter().map(|w| w.kind));
                write!(f, "there is no widget called `{found}`; there is {kinds}")
            }
            Reason::UnknownProperty {
                kind,
                found,
                accepted,
            } => {
                let names = listing(accepted.iter().map(|p| p.name));
                write!(f, "`{kind}` has no property `{found}`; it accepts {names}")
            }
            Reason::WrongType { kind, name, wanted } => {
                write!(f, "`{name}` on `{kind}` takes {wanted}")
            }
            Reason::NotAName {
                name,
                found,
                accepted,
            } => {
                let names = listing(accepted.iter());
                write!(f, "`{found}` is not a {name}; try {names}")
            }
            Reason::UnexpectedChild { parent, found } => {
                write!(f, "a `{parent}` has no use for a `{found}` inside it")
            }
            Reason::UnknownMessage { found } => write!(
                f,
                "the application does not know a message called `{found}`"
            ),
            Reason::WrongMessage { found, wanted } => write!(
                f,
                "`{found}` resolved to the wrong kind of message; this one needs {wanted}"
            ),
            Reason::Asset { path } => {
                write!(f, "the application could not load `{path}`")
            }
            Reason::DuplicateName { name } => {
                write!(f, "two nodes are called `{name}`; a name identifies one")
            }
            Reason::TwoFocuses => write!(
                f,
                "two nodes ask for the caret with `focus=#true`; only one can have it"
            ),
            Reason::TreeRefused => write!(f, "the tree would not take this node"),
            Reason::TooDeep { limit } => write!(
                f,
                "this form nests more than {limit} deep, which is past what a \
                 form is and into what a stack overflow is"
            ),
            Reason::TooLarge { limit } => {
                write!(
                    f,
                    "a form file is at most {limit} bytes; this one is larger"
                )
            }
            Reason::NoSuchNode { path } => write!(f, "there is no node at {path:?}"),
            Reason::NoArgument => write!(
                f,
                "this node is written without an argument; set the property instead"
            ),
            Reason::WrongKind { name, holds, given } => write!(
                f,
                "`{name}` holds {holds}; putting {given} there would make a form that will not load"
            ),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_position_counts_from_one_the_way_an_editor_does() {
        let source = "abc\ndef\n";
        assert_eq!(At::of(source, 0), At { line: 1, column: 1 });
        assert_eq!(At::of(source, 2), At { line: 1, column: 3 });
        assert_eq!(At::of(source, 4), At { line: 2, column: 1 });
        assert_eq!(At::of(source, 6), At { line: 2, column: 3 });
    }

    #[test]
    fn a_column_counts_characters_rather_than_bytes() {
        // Four bytes before the `x`, but two characters.
        let source = "æø x";
        assert_eq!(At::of(source, 5), At { line: 1, column: 4 });
    }

    #[test]
    fn a_position_past_the_end_lands_at_the_end() {
        let source = "ab";
        assert_eq!(At::of(source, 99), At { line: 1, column: 3 });
    }

    #[test]
    fn a_long_list_of_names_is_cut_short_rather_than_dumped() {
        let many: Vec<String> = (0..40).map(|n| format!("name{n:02}")).collect();
        let rendered = listing(many.iter());
        assert!(rendered.contains("and 28 more"), "{rendered}");
        assert!(rendered.len() < 200, "{rendered}");
    }
}
