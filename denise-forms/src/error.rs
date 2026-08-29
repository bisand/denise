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
    ///
    /// ```
    /// # use denise_forms::At;
    /// let source = "form \"F\" version=1\n    label \"æøå\" x=0\n";
    ///
    /// assert_eq!(At::of(source, 0), At { line: 1, column: 1 });
    /// // Characters, not bytes, so a column points where a caret would be even
    /// // on a line with an `æ` in it.
    /// assert_eq!(At::of(source, source.len() - 1).line, 2);
    /// ```
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
    /// A property the `form` node does not carry — often one that belongs to a
    /// different kind of form.
    ///
    /// Separate from [`Reason::UnknownProperty`] because what a form accepts
    /// depends on its kind, so the list has to be built rather than pointed at.
    UnknownFormProperty {
        /// What kind of form it said it was.
        kind: &'static str,
        /// What the file said.
        found: String,
        /// Everything a form of that kind does accept.
        accepted: Vec<&'static str>,
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
    /// A designer's placeholder content written where the engine would load it.
    ///
    /// A `table`'s `row`s and a `timeline`'s `event`s belong in that node's
    /// `design { … }` block, where every build but a designer's skips them. Left
    /// outside, they would ship to a kiosk — so this is an error rather than a
    /// thing quietly ignored, for the same reason an unknown property is.
    PlaceholderOutside {
        /// The widget's kind.
        kind: String,
        /// The child node's name.
        found: String,
    },
    /// One message name is used with two different payload shapes, so no single
    /// enum variant can serve both.
    ///
    /// Only [`codegen`](crate::codegen) raises this. The engine is happy to
    /// resolve one name twice, because the application's `match` answers each
    /// call separately; a generated enum cannot, because `Greet` is either a
    /// variant or a `fn(bool) -> M` and not both.
    PayloadClash {
        /// The name used twice.
        found: String,
        /// What it was first seen as.
        first: &'static str,
        /// And then as.
        then: &'static str,
    },
    /// A name in the file cannot be turned into a Rust identifier.
    ///
    /// Only [`codegen`](crate::codegen). A form loads perfectly well with a node
    /// called `2`; a struct field cannot be called that.
    NotAnIdentifier {
        /// What the file called it.
        found: String,
        /// Why it will not do.
        because: &'static str,
    },
    /// Two names in the file become one Rust identifier.
    ///
    /// Only [`codegen`](crate::codegen). `full-name` and `full_name` are two
    /// nodes to a form file and one field to Rust.
    Collides {
        /// The second name to arrive.
        found: String,
        /// The one already there.
        with: String,
        /// What they both became.
        spelled: String,
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
    /// A brace has no partner.
    ///
    /// Refused by the same byte scan that counts depth, and for the same
    /// reason: a file whose braces do not balance cannot parse whatever the
    /// parser does with it, and `kdl` can spend an unbounded amount of time
    /// discovering that. Saying it up front costs one pass and gives a better
    /// position than the recovery would.
    Unbalanced {
        /// `true` for a `{` that is never closed, `false` for a `}` that
        /// closes nothing.
        open: bool,
    },
    /// The file nests commented-out children blocks past what the parser can
    /// be asked to read.
    ///
    /// `kdl` takes time that doubles with every commented-out block nested
    /// inside another: a hundred bytes of them is twenty seconds, and three
    /// hundred is longer than anyone will wait. So this is refused by a byte
    /// scan before the parser is handed the file at all — the same treatment,
    /// and for the same reason, as nesting past
    /// [`MAX_DEPTH`](crate::MAX_DEPTH), which overflows its stack. See
    /// [`MAX_COMMENTED_DEPTH`](crate::MAX_COMMENTED_DEPTH).
    CommentedTooDeep {
        /// The limit, in levels.
        limit: usize,
    },
    /// The parse was still running when the caller's deadline passed, so it
    /// was abandoned and the file was not read.
    ///
    /// Only [`Form::parse_within`](crate::Form::parse_within) raises this, and
    /// only a caller who asked for a deadline can get it. It says nothing about
    /// the file beyond how long it was taking: a hostile one that has found a
    /// corner of `kdl` that takes exponential time looks exactly like a
    /// legitimate one on a machine that is too slow for the number chosen. The
    /// position is the top of the file, because nothing in it has been read.
    TooSlow {
        /// The time the caller allowed.
        limit: core::time::Duration,
    },
    /// A bounded parse could not be started, so the file was not read at all.
    ///
    /// [`Form::parse_within`](crate::Form::parse_within) works on a thread it
    /// can walk away from, and this says there was no such thread to be had:
    /// either [`MAX_ABANDONED`](crate::MAX_ABANDONED) earlier parses are still
    /// running past their deadlines — a wedged thread each, and the point of
    /// the limit is that a machine does not fill up with them — or the system
    /// refused a thread outright.
    NoThread {
        /// How many earlier parses are still running past their deadline.
        abandoned: usize,
    },
    /// The parser could not keep the file byte-for-byte.
    ///
    /// Everything this crate does — undo, the designer's save, a text editor
    /// alongside — stands on [`Form::text`](crate::Form::text) reproducing what
    /// was opened, and a file that cannot be reproduced would silently lose
    /// bytes on the first save. Refusing it is the honest alternative.
    ///
    /// The known causes are all one thing — kdl dropping the trivia between a
    /// closing brace and the next node, whether that is trailing whitespace or
    /// a comment written on the brace's line — and all of it is put back before
    /// this is ever raised. So reaching this means a shape nobody has seen yet,
    /// which is what the fuzz target `parse_form` is hunting for.
    NotPreserved,
    /// An edit named a node that is not there.
    NoSuchNode {
        /// The child path that went nowhere.
        path: Vec<usize>,
    },
    /// A move would have put a node inside itself.
    ///
    /// Or would have moved the form, which is the document rather than a node in
    /// it. Both are the same mistake — a tree cannot contain its own root — and
    /// both come from a drag that ended where it should not have been allowed to.
    IntoItself {
        /// The node the move named.
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
            Reason::UnknownFormProperty {
                kind,
                found,
                accepted,
            } => {
                let names = listing(accepted.iter());
                write!(
                    f,
                    "a `{kind}` form has no property `{found}`; it accepts {names}"
                )
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
            Reason::PayloadClash { found, first, then } => write!(
                f,
                "`{found}` is used as {first} and as {then}; one name cannot generate both"
            ),
            Reason::NotAnIdentifier { found, because } => {
                write!(f, "`{found}` cannot name anything in Rust: {because}")
            }
            Reason::Collides {
                found,
                with,
                spelled,
            } => write!(f, "`{found}` and `{with}` are both `{spelled}` in Rust"),
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
            Reason::PlaceholderOutside { kind, found } => write!(
                f,
                "a `{found}` is placeholder content, so it belongs in this \
                 {kind}'s `design {{ … }}` block; written here it would be \
                 loaded on a panel that has its own"
            ),
            Reason::Unbalanced { open } => {
                if *open {
                    write!(f, "this `{{` is never closed")
                } else {
                    write!(f, "this `}}` closes nothing")
                }
            }
            Reason::CommentedTooDeep { limit } => write!(
                f,
                "this nests commented-out blocks more than {limit} deep, and \
                 every level of that doubles what reading the file costs"
            ),
            Reason::TooSlow { limit } => write!(
                f,
                "this form was taking longer than {limit:?} to parse, so it \
                 was abandoned unread"
            ),
            Reason::NoThread { abandoned } => {
                if *abandoned >= crate::MAX_ABANDONED {
                    write!(
                        f,
                        "{abandoned} earlier parses are still running past \
                         their deadline and cannot be stopped, so this form \
                         was not started; restart to clear them"
                    )
                } else {
                    write!(
                        f,
                        "this form needs a thread of its own to be parsed \
                         under a deadline, and the system would not give it one"
                    )
                }
            }
            Reason::NotPreserved => write!(
                f,
                "the parser cannot keep this file byte-for-byte, so saving it \
                 would corrupt it; the difference starts here"
            ),
            Reason::TooLarge { limit } => {
                write!(
                    f,
                    "a form file is at most {limit} bytes; this one is larger"
                )
            }
            Reason::NoSuchNode { path } => write!(f, "there is no node at {path:?}"),
            Reason::IntoItself { path } => {
                write!(f, "a node cannot go inside itself; {path:?} was asked to")
            }
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
