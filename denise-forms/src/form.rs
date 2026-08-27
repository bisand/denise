//! The file, parsed but not yet built.

use denise::{Rect, Role, Size, Theme, theme};
use denise_ui::Side;
use kdl::{KdlDocument, KdlEntry, KdlEntryFormat, KdlNode, KdlNodeFormat, KdlValue};

use crate::error::{At, Error, Reason};

/// The schema version this crate reads.
///
/// A file may add properties within a major version — an engine older than the
/// file reports the added one as unknown, which is the same message a typo gets
/// and has the same fix. A file whose `version` is *higher* than this is refused
/// by number rather than misread.
pub const VERSION: u64 = 1;

/// How deep a form may nest.
///
/// A form is a screen. Anything approaching this is a generated file or a
/// malicious one.
///
/// This is checked **before the file reaches the parser**, and that is not
/// belt-and-braces: `kdl` is a recursive-descent parser and overflows the stack
/// on a few hundred nested nodes, which a `Result` cannot catch and a panic
/// handler cannot either. A form arrives from a designer, a clipboard or a
/// download, so bounding it is this crate's job and not its caller's.
pub const MAX_DEPTH: usize = 64;

/// How large a form file may be.
///
/// Generous for a screen — the reference form is under five kilobytes — and small
/// enough that a panel with a few megabytes of headroom cannot be talked into
/// exhausting them by something claiming to be a form.
pub const MAX_SOURCE: usize = 1 << 22;

/// How deeply a form may nest a **commented-out children block** — a `{ … }`
/// belonging to a node a `/-` has commented out.
///
/// Its own limit because neither [`MAX_SOURCE`] nor [`MAX_DEPTH`] bounds it in
/// any useful way. `kdl` takes time that **doubles with every level** of one of
/// these inside another: twenty levels is about a hundred bytes and twenty
/// seconds, thirty is six hours, and the sixty-four [`MAX_DEPTH`] would permit
/// is longer than the universe has been running. Found by the fuzz target
/// `parse_form`, which kept reporting three-kilobyte inputs that took a second
/// and a half to *fail* on.
///
/// One level is kept, because commenting a widget and its children out is a
/// real thing to do while editing. Two is refused, because a block commented
/// out inside a block that is already commented out changes nothing about what
/// the file means — the fix is deleting an inner `/-` that was doing no work —
/// and every one of the slow inputs the fuzzer found is past this line while
/// every form in this repository is nowhere near it.
///
/// This bounds the shapes that have been found, and it is not a bound on the
/// parser: `kdl` 6.7.1 has more exponential corners than this one, and a caller
/// reading a form it did not write should still bound how long a parse may take.
/// See `fuzz/README.md`.
pub const MAX_COMMENTED_DEPTH: usize = 1;

/// What a form is for.
///
/// The engine reports it and opens nothing: whether a dialog is a pushed scene or
/// a modal window is the application's decision, and it differs by machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FormKind {
    /// The root tree of a `Ui` — a panel's whole surface.
    Screen,
    /// A desktop window.
    Window,
    /// A modal: `Ui::push_scene` on a panel, a modal window on a desktop.
    Dialog,
    /// `Ui::push_drawer`.
    Drawer,
    /// `Ui::push_shelf`.
    Shelf,
    /// A subtree with no root of its own, for reuse inside other forms.
    Fragment,
}

impl FormKind {
    /// Every kind, in the spelling a form file uses.
    pub const NAMES: &'static [&'static str] =
        &["screen", "window", "dialog", "drawer", "shelf", "fragment"];

    /// ```
    /// # use denise_forms::FormKind;
    /// assert!(FormKind::Dialog.what().contains("modal"));
    /// // Every kind has one, and no two share it.
    /// assert_ne!(FormKind::Drawer.what(), FormKind::Shelf.what());
    /// ```
    /// One line on what this kind is for.
    ///
    /// Here rather than in the designer because it is a fact about the format:
    /// the CLI prints it, the designer offers it when a form is being made, and
    /// neither of them should be keeping its own copy.
    pub const fn what(self) -> &'static str {
        match self {
            Self::Screen => "A panel's whole surface: the root of a `Ui`.",
            Self::Window => "A desktop window, with a title bar and a size somebody can drag.",
            Self::Dialog => "A modal: a pushed scene on a panel, a modal window on a desktop.",
            Self::Drawer => "A panel that slides in from an edge, over a dimmed screen.",
            Self::Shelf => "A bar that slides in from an edge, with nothing dimmed behind it.",
            Self::Fragment => "A subtree with no root of its own, for reuse inside other forms.",
        }
    }

    /// ```
    /// # use denise_forms::FormKind;
    /// # use denise_ui::Side;
    /// // A drawer is a side panel; a shelf is a bar.
    /// assert_eq!(FormKind::Drawer.default_side(), Side::Before);
    /// assert_eq!(FormKind::Shelf.default_side(), Side::Below);
    /// ```
    /// Which edge one of these comes in from when the file does not say.
    pub const fn default_side(self) -> Side {
        match self {
            Self::Shelf => Side::Below,
            _ => Side::Before,
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "screen" => FormKind::Screen,
            "window" => FormKind::Window,
            "dialog" => FormKind::Dialog,
            "drawer" => FormKind::Drawer,
            "shelf" => FormKind::Shelf,
            "fragment" => FormKind::Fragment,
            _ => return None,
        })
    }
}

/// Whether a form may be drawn at a size other than the one it was designed at.
///
/// Scaling is not always right, and the form is the thing that knows. A dial
/// designed against a 1:1 photographic background, a layout whose text must stay
/// a legal minimum size, a panel whose touch targets are already at the smallest
/// a gloved finger can hit — each of those is a form that should be drawn at its
/// design size and centred, not stretched to fit. So it is declared rather than
/// assumed, and the default is the one every form written before this property
/// existed already had.
///
/// This is a **deployment** concern, applied once on the way in.
/// [`anchors`](crate::NODE_PROPERTIES) are a *design* concern, resolved by the
/// tree at every reflow. They are different tools and they compose: a form may
/// use either, both or neither.
///
/// ```
/// # use denise_forms::{Form, Scaling};
/// let form = Form::parse(r#"form "F" version=1 width=100 height=100 { }"#)?;
/// assert_eq!(form.scaling(), Scaling::None, "the default is what was always true");
/// # Ok::<(), denise_forms::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Scaling {
    /// Never scaled. Drawn at its design size, centred in whatever it is given.
    #[default]
    None,
    /// One factor on both axes — `min(target.w / design.w, target.h / design.h)`
    /// — so nothing distorts, and the leftover is a margin on one axis.
    Proportional,
    /// A factor per axis, filling the surface. Distorts, and is occasionally
    /// exactly what a signage layout wants.
    Stretch,
}

impl Scaling {
    /// Every one, in the spelling a form file uses.
    pub const NAMES: &'static [&'static str] = &["none", "proportional", "stretch"];

    /// ```
    /// # use denise_forms::Scaling;
    /// assert!(Scaling::None.what().contains("design size"));
    /// assert_ne!(Scaling::Proportional.what(), Scaling::Stretch.what());
    /// ```
    /// One line on what this one does.
    ///
    /// Here rather than in the designer for the same reason as
    /// [`FormKind::what`]: it is a fact about the format, and two copies of a
    /// sentence drift.
    pub const fn what(self) -> &'static str {
        match self {
            Self::None => "Never scaled: drawn at its design size, in the middle.",
            Self::Proportional => "Scaled to fit by one factor, with a margin on the long axis.",
            Self::Stretch => "Scaled per axis to fill the surface, distorting if it must.",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "none" => Self::None,
            "proportional" => Self::Proportional,
            "stretch" => Self::Stretch,
            _ => return None,
        })
    }
}

/// Where a form goes on a surface, and by how much it is multiplied to get there.
///
/// What [`Form::fit`] works out and [`Form::build_fitted`] then applies. Held as
/// a value rather than done in one call because the application needs the parts
/// separately: [`Placement::rect`] is where to put the panel the form is built into,
/// and [`Placement::uniform`] is what the theme has to be scaled by at `Ui::new` —
/// **which is not optional**, or the widgets are the old size inside the new
/// rectangles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// The horizontal factor.
    pub x: f32,
    /// The vertical factor.
    pub y: f32,
    /// Where the form's own rectangle lands in the surface it was fitted to,
    /// already scaled and already centred.
    pub rect: Rect,
}

impl Placement {
    /// The one factor that everything which is not a rectangle scales by: text
    /// sizes, border widths, row heights, and the theme's own metrics.
    ///
    /// The **smaller** of the two, so that a stretched layout never grows text
    /// taller than the axis that had least room to give. Equal to either of them
    /// whenever the fit is uniform, which is every fit except
    /// [`Scaling::Stretch`].
    ///
    /// ```
    /// # use denise::Size;
    /// # use denise_forms::Form;
    /// let form = Form::parse(
    ///     r#"form "F" version=1 width=100 height=100 scaling=stretch { }"#,
    /// )?;
    /// let fit = form.fit(Size::new(400, 200));
    /// assert_eq!((fit.x, fit.y), (4.0, 2.0));
    /// assert_eq!(fit.uniform(), 2.0, "text follows the tighter axis");
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    #[must_use]
    pub fn uniform(self) -> f32 {
        if self.x < self.y { self.x } else { self.y }
    }
}

/// The themes a form may name.
pub const THEMES: &[&str] = &["dark", "light", "high-contrast"];

/// A property's value, as a form file writes it.
///
/// Smaller than KDL's own set, because a `.dform` property is one of these and
/// nothing else. The spelling is part of it: `role=primary` and
/// `text="primary"` hold the same string and are not the same line, so
/// [`Name`](Literal::Name) and [`Text`](Literal::Text) are separate.
#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    /// A quoted string: `text="Save"`.
    Text(String),
    /// A bare name, which is how an enum is written: `role=primary`.
    ///
    /// Quoted anyway if the name is not something KDL would read back bare, so
    /// this can never produce a file that stops parsing.
    Name(String),
    /// `#true` or `#false`.
    Flag(bool),
    /// A whole number.
    Int(i64),
    /// A real number.
    Float(f64),
    /// Exactly this text, character for character, as it stood in the file.
    ///
    /// What [`Form::apply`] hands back for a value it replaced, and the reason
    /// undo is byte-exact rather than merely correct: `1_000`, `0x10`, `70.0`
    /// and `#"a raw string"#` are all values a typed variant could carry, and
    /// none of them would be written the same way twice.
    ///
    /// One built by hand is checked: it has to be the text of a single value.
    Verbatim(String),
}

impl Literal {
    /// The spelling is the difference, and the file keeps it: `role=primary`
    /// and `text="primary"` hold the same string and are not the same line.
    ///
    /// ```
    /// # use denise_forms::{Edit, Form, Literal};
    /// let mut form = Form::parse(r#"form "F" version=1 width=99 height=99 { label "Hi" x=0 y=0 w=9 h=9 }"#)?;
    ///
    /// form.apply(Edit::property(&[0], "text", Some(Literal::text("Hello"))))?;
    /// form.apply(Edit::property(&[0], "role", Some(Literal::name("primary"))))?;
    ///
    /// assert!(form.text().contains(r#"text="Hello""#), "{}", form.text());
    /// assert!(form.text().contains("role=primary"), "{}", form.text());
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    /// A quoted string.
    pub fn text(text: impl Into<String>) -> Self {
        Literal::Text(text.into())
    }

    /// See [`Literal::text`].
    /// A bare name.
    pub fn name(name: impl Into<String>) -> Self {
        Literal::Name(name.into())
    }

    /// The value this stands for, and the text to write for it.
    fn parts(&self) -> Result<(KdlValue, String), Error> {
        Ok(match self {
            Literal::Text(text) => (KdlValue::String(text.clone()), quoted(text)),
            // `KdlValue`'s own rendering writes a plain identifier bare and
            // quotes anything else, which is this variant's rule exactly.
            Literal::Name(name) => {
                let value = KdlValue::String(name.clone());
                let repr = value.to_string();
                (value, repr)
            }
            Literal::Flag(flag) => {
                let value = KdlValue::Bool(*flag);
                let repr = value.to_string();
                (value, repr)
            }
            Literal::Int(number) => {
                let value = KdlValue::Integer(i128::from(*number));
                let repr = value.to_string();
                (value, repr)
            }
            Literal::Float(number) => {
                let value = KdlValue::Float(*number);
                let repr = value.to_string();
                (value, repr)
            }
            Literal::Verbatim(text) => (one_value(text)?, text.clone()),
        })
    }

    /// What sort of thing this is, for the rule that a number and a string do
    /// not replace each other.
    fn class(&self) -> Result<Class, Error> {
        Ok(match self {
            Literal::Text(_) | Literal::Name(_) => Class::Text,
            Literal::Flag(_) => Class::Flag,
            Literal::Int(_) | Literal::Float(_) => Class::Number,
            Literal::Verbatim(text) => Class::of(&one_value(text)?),
        })
    }
}

/// The sorts of value a property may hold.
///
/// Coarser than [`Literal`] on purpose: `value=70` becoming `value=70.5` is an
/// ordinary edit, and `placeholder="Ada"` becoming `placeholder=70` is a form
/// that will not load. One kind of change is worth refusing and the other is
/// not, and this is the line between them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Text,
    Number,
    Flag,
    Nothing,
}

impl Class {
    fn of(value: &KdlValue) -> Self {
        match value {
            KdlValue::String(_) => Class::Text,
            KdlValue::Integer(_) | KdlValue::Float(_) => Class::Number,
            KdlValue::Bool(_) => Class::Flag,
            KdlValue::Null => Class::Nothing,
        }
    }

    const fn noun(self) -> &'static str {
        match self {
            Class::Text => "a string",
            Class::Number => "a number",
            Class::Flag => "true or false",
            Class::Nothing => "nothing",
        }
    }
}

/// One reversible change to a form.
///
/// Applied with [`Form::apply`], which hands back the edit that undoes it. See
/// there for why an inverse is always knowable.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// Set a property, or take it away with `None`.
    Property {
        /// The node.
        path: Vec<usize>,
        /// The property.
        name: String,
        /// What to set it to, or `None` to remove it — which is what returning a
        /// property to its default means, since a default is not written.
        value: Option<Literal>,
    },
    /// Put a node, written as form-file text, among a parent's children.
    ///
    /// The text is a whole node with its own formatting, which is what makes this
    /// the inverse of a removal *and* what a paste from the clipboard is.
    Insert {
        /// The parent's path; empty for the form itself.
        parent: Vec<usize>,
        /// Where among its children.
        index: usize,
        /// The node.
        text: String,
    },
    /// Set a node's positional argument — the `"Hello"` in `label "Hello"`.
    ///
    /// Only ever *replaces* one. A node written without an argument does not
    /// grow one this way: an argument has to come before every property, and
    /// there is no shape of edit that puts something at the front of a line
    /// without rewriting the line. Setting the matching property is what an
    /// editor does instead, and means the same thing to the engine.
    Argument {
        /// The node.
        path: Vec<usize>,
        /// What to put there.
        value: Literal,
    },
    /// Take a node out and put it back under another parent.
    ///
    /// Reordering among siblings and reparenting are the same edit: both take a
    /// node out and put it back somewhere, and doing it as one keeps it to one
    /// step on an undo stack.
    ///
    /// A node that changes depth is **re-indented** — every line of it, so the
    /// children come along — because a file whose nesting and whose indentation
    /// disagree is a file somebody has to fix by hand. Moving it back re-indents
    /// it back, so an undo is still byte-for-byte.
    Move {
        /// The node now.
        from: Vec<usize>,
        /// The parent it goes under; empty for the form itself.
        to: Vec<usize>,
        /// Where among that parent's children.
        index: usize,
    },
    /// Take a node, and everything under it, out.
    Remove {
        /// The node.
        path: Vec<usize>,
    },
    /// Several edits as one.
    ///
    /// Applied in order and undone in reverse, which is what makes a drag that
    /// moved *and* resized a single step on an undo stack rather than four. If
    /// any of them fails the ones already applied are put back, so a compound
    /// edit either happens or does not.
    Many(Vec<Edit>),
    /// Swap a node for another, written as form-file text.
    ///
    /// The exact inverse of anything that cannot be undone by putting a value
    /// back — taking a property *away* is the case: a property re-added by name
    /// lands at the end of the line rather than where it was, so the values would
    /// come back right and the line would not. Restoring the node's own text
    /// restores its order too.
    Replace {
        /// The node.
        path: Vec<usize>,
        /// What to put there.
        text: String,
    },
}

impl Edit {
    /// Sets or clears a whole-number property.
    ///
    /// The common one by a long way: every rectangle a drag writes is four of
    /// these.
    /// ```
    /// # use denise_forms::{Edit, Form};
    /// let mut form = Form::parse(r#"form "F" version=1 width=99 height=99 { label "Hi" x=0 y=0 w=9 h=9 }"#)?;
    ///
    /// form.apply(Edit::number(&[0], "x", Some(24)))?;
    /// assert_eq!(form.property(&[0], "x").as_deref(), Some("24"));
    ///
    /// // `None` takes it out of the file, which is what a default is.
    /// form.apply(Edit::number(&[0], "x", None))?;
    /// assert_eq!(form.property(&[0], "x"), None);
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    pub fn number(path: &[usize], name: &str, value: Option<i64>) -> Self {
        Edit::property(path, name, value.map(Literal::Int))
    }

    /// The path is child indices from the `form` node down, and **the empty path
    /// is the form itself** — its size, its kind, its theme.
    ///
    /// ```
    /// # use denise_forms::{Edit, Form, Literal};
    /// let mut form = Form::parse(r#"form "F" version=1 width=320 height=240 { label "Hi" x=0 y=0 w=9 h=9 }"#)?;
    ///
    /// form.apply(Edit::property(&[], "width", Some(Literal::Int(640))))?;
    /// assert_eq!(form.size(), denise::Size::new(640, 240));
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    /// Sets or clears a property.
    pub fn property(path: &[usize], name: &str, value: Option<Literal>) -> Self {
        Edit::Property {
            path: path.to_vec(),
            name: name.to_string(),
            value,
        }
    }

    /// A `label "Heading"` keeps its text there rather than in a `text=`
    /// property, and so does the form's own title.
    ///
    /// ```
    /// # use denise_forms::{Edit, Form};
    /// let mut form = Form::parse(r#"form "F" version=1 width=99 height=99 { label "Hi" x=0 y=0 w=9 h=9 }"#)?;
    ///
    /// form.apply(Edit::argument(&[0], "Hello"))?;
    /// assert_eq!(form.argument(&[0]).as_deref(), Some("Hello"));
    ///
    /// // The form's title is its argument too.
    /// form.apply(Edit::argument(&[], "Greeting"))?;
    /// assert_eq!(form.title(), "Greeting");
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    /// Sets a node's positional argument to a string.
    pub fn argument(path: &[usize], text: impl Into<String>) -> Self {
        Edit::Argument {
            path: path.to_vec(),
            value: Literal::Text(text.into()),
        }
    }

    /// `index` is the position **after** the node has been taken out, which is
    /// the part that is easy to get wrong: removing `[1]` moves `[3]` to `[2]`.
    ///
    /// ```
    /// # use denise_forms::{Edit, Form};
    /// let mut form = Form::parse(
    ///     "form \"F\" version=1 width=99 height=99 {\n    label \"a\" x=0 y=0 w=9 h=9\n    panel name=box x=0 y=9 w=9 h=9\n}\n",
    /// )?;
    ///
    /// // The label into the panel, which grows the braces it did not have.
    /// form.apply(Edit::move_to(&[0], &[1], 0))?;
    /// assert!(form.text().contains("panel name=box x=0 y=9 w=9 h=9 {"), "{}", form.text());
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    /// Moves a node under another parent, or to another place among its
    /// siblings.
    pub fn move_to(from: &[usize], to: &[usize], index: usize) -> Self {
        Edit::Move {
            from: from.to_vec(),
            to: to.to_vec(),
            index,
        }
    }

    /// Its children go with it, and so does the comment written above it — the
    /// node's leading trivia is part of the node, which is what makes undoing a
    /// removal put the comment back.
    ///
    /// ```
    /// # use denise_forms::{Edit, Form};
    /// let source = "form \"F\" version=1 width=99 height=99 {\n    // why\n    label \"a\" x=0 y=0 w=9 h=9\n}\n";
    /// let mut form = Form::parse(source)?;
    ///
    /// let undo = form.apply(Edit::remove(&[0]))?;
    /// assert!(!form.text().contains("why"));
    ///
    /// form.apply(undo)?;
    /// assert_eq!(form.text(), source, "the comment came back with the node");
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    /// Removes a node.
    pub fn remove(path: &[usize]) -> Self {
        Edit::Remove {
            path: path.to_vec(),
        }
    }
}

/// One node of a form file, as the file writes it.
///
/// [`Form::written`] is where these come from, and says why they are not
/// [`Placed`](crate::Placed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    /// Child indices from the `form` node down to this one. Empty for the form.
    pub path: Vec<usize>,
    /// The node's name in the file — `label`, `panel`, `form`. Whatever the file
    /// says, which is not necessarily a widget this toolkit has.
    pub kind: String,
    /// What `name=` gave it, if it gave one.
    pub name: Option<String>,
    /// Its positional argument — the `"Hello"` in `label "Hello"` — if it has
    /// one. The other half of saying which node this is, to somebody who has
    /// the file open and never named it.
    pub argument: Option<String>,
    /// The node's own entries, spelled canonically: its kind, then every
    /// argument and property in the order the file writes them, and nothing
    /// else. No children, no comment above it, and none of the spacing between
    /// any of it — so a file somebody realigned by hand does not read as though
    /// every node in it changed. Every string is quoted, whether the file
    /// bothered to or not, for the same reason.
    pub line: String,
}

/// A parsed form file.
///
/// Holds the document rather than a value taken from it — comments, spacing and
/// entry order included — because the designer edits this and saves it back, and
/// a save that reformats what nobody touched is a save people learn not to make.
#[derive(Clone, Debug)]
pub struct Form {
    source: String,
    doc: KdlDocument,
}

impl Form {
    /// Parses a form file.
    ///
    /// The source is kept, and what is kept is checked: the parsed document is
    /// written back out and compared to the input, so a file that would not
    /// reproduce is refused with [`Reason::NotPreserved`] rather than accepted
    /// and corrupted on the first save.
    ///
    /// ```
    /// # use denise_forms::{Form, FormKind};
    /// let form = Form::parse(r#"
    ///     form "Hello" version=1 kind=screen width=460 height=260
    /// "#)?;
    /// assert_eq!(form.title(), "Hello");
    /// assert_eq!(form.kind(), FormKind::Screen);
    /// assert_eq!(form.size(), denise::Size::new(460, 260));
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    pub fn parse(source: &str) -> Result<Self, Error> {
        // Before the parser, not after: see `MAX_DEPTH`.
        if source.len() > MAX_SOURCE {
            return Err(Error::new(
                At::START,
                Reason::TooLarge { limit: MAX_SOURCE },
            ));
        }
        if let Some(refusal) = unparseable(source) {
            return Err(refusal.error(source));
        }

        let doc: KdlDocument = source.parse().map_err(|error: kdl::KdlError| {
            let first = error.diagnostics.first();
            let at = first.map_or(At::START, |d| At::of(source, d.span.offset()));
            let message = first.map_or_else(
                || String::from("this is not a KDL document"),
                |d| {
                    d.message
                        .clone()
                        .unwrap_or_else(|| d.help.clone().unwrap_or_default())
                },
            );
            let message = if message.is_empty() {
                String::from("this is not a KDL document")
            } else {
                message
            };
            Error::new(at, Reason::Syntax(message))
        })?;

        let mut doc = doc;
        restore_after_close(&mut doc, source);
        // What was accepted must be reproducible, or the first save silently
        // loses bytes. The repair above covers every way kdl is known to drop
        // trivia; anything it does not cover is refused here, at the first
        // byte that differs — and the fuzz target `parse_form` treats this
        // error as a finding, so the next lossy shape becomes a repair rather
        // than a refusal.
        let reproduced = doc.to_string();
        if reproduced != source {
            let at = source
                .bytes()
                .zip(reproduced.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| source.len().min(reproduced.len()));
            return Err(Error::new(At::of(source, at), Reason::NotPreserved));
        }

        let form = Self {
            source: source.to_string(),
            doc,
        };
        form.check_shape()?;
        Ok(form)
    }

    /// Everything that must be true before a single widget is built.
    ///
    /// Separate from building so that a file can be checked without an
    /// application, a theme or a display — which is what `denise-forms check`
    /// is, and what a form's own unit test wants.
    fn check_shape(&self) -> Result<(), Error> {
        let nodes = self.doc.nodes();
        let root = match nodes {
            [only] if only.name().value() == "form" => only,
            [] => {
                return Err(Error::new(
                    At::START,
                    Reason::NotAForm {
                        found: String::from("nothing"),
                    },
                ));
            }
            [first, ..] => {
                let found = if first.name().value() == "form" {
                    // A second top-level node: point at *it*, not at the form.
                    nodes[1].name().value().to_string()
                } else {
                    first.name().value().to_string()
                };
                let offender = if first.name().value() == "form" {
                    &nodes[1]
                } else {
                    first
                };
                return Err(Error::new(
                    self.at(offender.span().offset()),
                    Reason::NotAForm { found },
                ));
            }
        };

        let version = root
            .get("version")
            .and_then(KdlValue::as_integer)
            .and_then(|v| u64::try_from(v).ok())
            .ok_or_else(|| Error::new(self.at_node(root), Reason::Version))?;
        if version > VERSION {
            return Err(Error::new(
                self.at_node(root),
                Reason::FromTheFuture {
                    wanted: version,
                    understood: VERSION,
                },
            ));
        }

        for axis in ["width", "height"] {
            if root.get(axis).and_then(KdlValue::as_integer).is_none() {
                return Err(Error::new(
                    self.at_node(root),
                    Reason::Missing {
                        kind: String::from("form"),
                        name: if axis == "width" { "width" } else { "height" },
                    },
                ));
            }
        }

        let kind = self
            .named(root, "kind", FormKind::NAMES, FormKind::from_name)?
            .unwrap_or(FormKind::Screen);
        self.named(root, "theme", THEMES, |n| THEMES.contains(&n).then_some(()))?;

        // A property the form node does not have is a mistake, and every widget
        // node has been told so since the beginning — the form node was the one
        // place a typo went quietly into the file and stayed there. What counts
        // is the descriptor, and the descriptor knows which kind it is: a
        // `resizable` on a screen is not an unused property, it is a window's
        // property on something that is not a window, and the message says so.
        for entry in root.entries() {
            let Some(name) = entry.name() else {
                continue;
            };
            let name = name.value();
            if name == "version" || crate::build::form_property(kind, name).is_some() {
                continue;
            }
            let accepted: Vec<&'static str> = crate::build::FORM_PROPERTIES
                .iter()
                .chain(crate::build::kind_properties(kind))
                .map(|property| property.name)
                .collect();
            return Err(Error::new(
                self.at(entry.span().offset()),
                Reason::UnknownFormProperty {
                    kind: FormKind::NAMES[kind as usize],
                    found: name.to_string(),
                    accepted,
                },
            ));
        }

        // What comes in from an edge has to say how far, or there is nothing for
        // the engine to slide and nothing for a designer to draw. Its other axis
        // is the surface it comes in over, which is what `width` and `height`
        // already are.
        if matches!(kind, FormKind::Drawer | FormKind::Shelf)
            && root.get("extent").and_then(KdlValue::as_integer).is_none()
        {
            return Err(Error::new(
                self.at_node(root),
                Reason::Missing {
                    kind: String::from(FormKind::NAMES[kind as usize]),
                    name: "extent",
                },
            ));
        }
        self.named(root, "side", denise_ui::widgets::SIDES, |n| {
            denise_ui::widgets::describe::side_from_name(n)
        })?;
        if let Some(background) = root.get("background") {
            let name = background.as_string().ok_or_else(|| {
                Error::new(
                    self.at_node(root),
                    Reason::WrongType {
                        kind: "form",
                        name: String::from("background"),
                        wanted: "one of the listed names",
                    },
                )
            })?;
            if denise_ui::widgets::describe::role_from_name(name).is_none() {
                return Err(Error::new(
                    self.at_node(root),
                    Reason::NotAName {
                        name: String::from("colour role"),
                        found: name.to_string(),
                        accepted: denise_ui::widgets::ROLES,
                    },
                ));
            }
        }
        Ok(())
    }

    /// Reads an optional property that must be one of a fixed set of names.
    fn named<T>(
        &self,
        node: &KdlNode,
        property: &'static str,
        accepted: &'static [&'static str],
        parse: impl Fn(&str) -> Option<T>,
    ) -> Result<Option<T>, Error> {
        let Some(value) = node.get(property) else {
            return Ok(None);
        };
        let name = value.as_string().ok_or_else(|| {
            Error::new(
                self.at_node(node),
                Reason::WrongType {
                    kind: "form",
                    name: property.to_string(),
                    wanted: "one of the listed names",
                },
            )
        })?;
        parse(name)
            .ok_or_else(|| {
                Error::new(
                    self.at_node(node),
                    Reason::NotAName {
                        name: property.to_string(),
                        found: name.to_string(),
                        accepted,
                    },
                )
            })
            .map(Some)
    }

    pub(crate) fn at(&self, offset: usize) -> At {
        At::of(&self.source, offset)
    }

    pub(crate) fn at_node(&self, node: &KdlNode) -> At {
        self.at(node.span().offset())
    }

    /// The `form` node itself.
    pub(crate) fn root(&self) -> &KdlNode {
        self.doc
            .nodes()
            .first()
            .expect("checked at parse: exactly one `form` node")
    }

    /// What a form says about itself, and the defaults for what it does not say.
    ///
    /// ```
    /// # use denise_forms::{Form, FormKind};
    /// let form = Form::parse(
    ///     r#"form "Preferences" name=prefs version=1 kind=window width=520 height=340 theme=light background=base-200"#,
    /// )?;
    ///
    /// assert_eq!(form.title(), "Preferences");
    /// assert_eq!(form.name(), Some("prefs"));
    /// assert_eq!(form.version(), 1);
    /// assert_eq!(form.kind(), FormKind::Window);
    /// assert_eq!(form.size(), denise::Size::new(520, 340));
    /// assert_eq!(form.theme_name(), "light");
    /// assert_eq!(form.background(), denise::Role::Base200);
    /// assert_eq!(form.theme(), denise::theme::LIGHT);
    ///
    /// // Nothing written is the default: a window may be resized, and says
    /// // nothing about a smallest size.
    /// assert!(form.resizable());
    /// assert_eq!(form.min_size(), None);
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    /// The form's title — a window's title bar, and the designer's name for it.
    pub fn title(&self) -> &str {
        self.root()
            .entries()
            .iter()
            .find(|e| e.name().is_none())
            .and_then(|e| e.value().as_string())
            .unwrap_or_default()
    }

    /// See [`Form::title`] for what a form says about itself.
    /// The form's identifier, if it was given one.
    pub fn name(&self) -> Option<&str> {
        self.root().get("name").and_then(KdlValue::as_string)
    }

    /// See [`Form::title`] for what a form says about itself.
    /// The schema version the file declares.
    pub fn version(&self) -> u64 {
        self.root()
            .get("version")
            .and_then(KdlValue::as_integer)
            .and_then(|v| u64::try_from(v).ok())
            .expect("checked at parse")
    }

    /// See [`Form::title`] for what a form says about itself.
    /// What this form is for. [`FormKind::Screen`] unless the file says otherwise.
    pub fn kind(&self) -> FormKind {
        self.root()
            .get("kind")
            .and_then(KdlValue::as_string)
            .and_then(FormKind::from_name)
            .unwrap_or(FormKind::Screen)
    }

    /// See [`Form::title`] for what a form says about itself.
    /// Whether this form consents to being drawn at another size.
    ///
    /// [`Scaling::None`] unless the file says otherwise, because that is what
    /// every form written before the property existed already did.
    pub fn scaling(&self) -> Scaling {
        self.root()
            .get("scaling")
            .and_then(KdlValue::as_string)
            .and_then(Scaling::from_name)
            .unwrap_or_default()
    }

    /// How this form occupies a surface of some other size.
    ///
    /// Reads [`Form::scaling`] and does the arithmetic, so that the policy lives
    /// in the file and the multiplication lives here — rather than in every
    /// application that loads a form.
    ///
    /// ```
    /// # use denise::Size;
    /// # use denise_forms::Form;
    /// # use denise::Rect;
    /// let source = |scaling: &str| {
    ///     format!(r#"form "F" version=1 width=200 height=100 scaling={scaling} {{ }}"#)
    /// };
    ///
    /// // The default: its own size, in the middle of the surface.
    /// let fixed = Form::parse(&source("none"))?;
    /// let fit = fixed.fit(Size::new(400, 400));
    /// assert_eq!((fit.x, fit.y), (1.0, 1.0));
    /// assert_eq!(fit.rect, Rect::new(100, 150, 200, 100));
    ///
    /// // Proportional: as big as fits, letterboxed on the axis with room left.
    /// let fits = Form::parse(&source("proportional"))?;
    /// let fit = fits.fit(Size::new(400, 400));
    /// assert_eq!((fit.x, fit.y), (2.0, 2.0), "the tighter axis decides");
    /// assert_eq!(fit.rect, Rect::new(0, 100, 400, 200));
    ///
    /// // Stretch: the whole surface, whatever that does to the shape.
    /// let fills = Form::parse(&source("stretch"))?;
    /// let fit = fills.fit(Size::new(400, 400));
    /// assert_eq!((fit.x, fit.y), (2.0, 4.0));
    /// assert_eq!(fit.rect, Rect::from_size(Size::new(400, 400)));
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    pub fn fit(&self, surface: Size) -> Placement {
        let design = self.size();
        // A form of no size cannot be fitted to anything; it is drawn where it
        // is and the caller finds out from the empty rectangle.
        if design.width == 0 || design.height == 0 {
            return Placement {
                x: 1.0,
                y: 1.0,
                rect: Rect::ZERO,
            };
        }
        let full = (
            surface.width as f32 / design.width as f32,
            surface.height as f32 / design.height as f32,
        );
        let (x, y) = match self.scaling() {
            Scaling::None => (1.0, 1.0),
            Scaling::Proportional => {
                let both = if full.0 < full.1 { full.0 } else { full.1 };
                (both, both)
            }
            Scaling::Stretch => full,
        };
        // Scaled by its own edges from the origin, then centred in what is left
        // — so the two halves of the margin differ by at most a pixel and the
        // form is never a pixel wider than the arithmetic says.
        let scaled = Rect::from_size(design).scaled_by(x, y);
        Placement {
            x,
            y,
            rect: Rect::new(
                (surface.width as i32 - scaled.width) / 2,
                (surface.height as i32 - scaled.height) / 2,
                scaled.width,
                scaled.height,
            ),
        }
    }

    /// ```
    /// # use denise_forms::Form;
    /// let fixed = Form::parse(
    ///     r#"form "F" version=1 kind=window width=400 height=300 resizable=#false min-width=320 min-height=240"#,
    /// )?;
    /// assert!(!fixed.resizable());
    /// assert_eq!(fixed.min_size(), Some(denise::Size::new(320, 240)));
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    /// Whether a window form may be resized. `true` unless the file says not.
    ///
    /// Meaningless on any other kind, which is why the file is not allowed to
    /// say it on one.
    pub fn resizable(&self) -> bool {
        self.root()
            .get("resizable")
            .and_then(KdlValue::as_bool)
            .unwrap_or(true)
    }

    /// See [`Form::resizable`].
    /// The smallest a window form may be made, if it says.
    pub fn min_size(&self) -> Option<Size> {
        let axis = |name: &str| {
            self.root()
                .get(name)
                .and_then(KdlValue::as_integer)
                .and_then(|v| u32::try_from(v).ok())
        };
        match (axis("min-width"), axis("min-height")) {
            (None, None) => None,
            (width, height) => Some(Size::new(width.unwrap_or(0), height.unwrap_or(0))),
        }
    }

    /// ```
    /// # use denise_forms::Form;
    /// let asked = Form::parse(r#"form "F" version=1 kind=dialog width=380 height=170 dim=200"#)?;
    /// assert_eq!(asked.dim(), 200);
    ///
    /// let quiet = Form::parse(r#"form "F" version=1 kind=dialog width=380 height=170"#)?;
    /// assert_eq!(quiet.dim(), 160);
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    /// How dark the backdrop behind a dialog is, 0 to 255. `160` by default,
    /// which is what [`denise_ui::Ui::push_scene`] is usually given.
    pub fn dim(&self) -> u8 {
        self.root()
            .get("dim")
            .and_then(KdlValue::as_integer)
            .and_then(|value| u8::try_from(value).ok())
            .unwrap_or(160)
    }

    /// `width` and `height` are the surface it comes in *over*; [`extent`] is
    /// how far it comes in, and across the other axis it covers the surface.
    ///
    /// ```
    /// # use denise_forms::Form;
    /// # use denise_ui::Side;
    /// let drawer = Form::parse(r#"form "F" version=1 kind=drawer width=1024 height=600 extent=320"#)?;
    /// assert_eq!(drawer.side(), Side::Before);
    /// assert_eq!(drawer.extent(), 320);
    ///
    /// // A shelf is a bar rather than a side panel, so it comes in from below.
    /// let shelf = Form::parse(r#"form "F" version=1 kind=shelf width=1024 height=600 extent=180"#)?;
    /// assert_eq!(shelf.side(), Side::Below);
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    ///
    /// [`extent`]: Form::extent
    /// Which edge a drawer or a shelf comes in from.
    ///
    /// The defaults differ by kind and deliberately: a drawer is a side panel
    /// and a shelf is a bar, so they come in from different edges when nobody
    /// says.
    pub fn side(&self) -> Side {
        self.root()
            .get("side")
            .and_then(KdlValue::as_string)
            .and_then(denise_ui::widgets::describe::side_from_name)
            .unwrap_or_else(|| self.kind().default_side())
    }

    /// See [`Form::side`].
    /// How far a drawer or a shelf comes in, in logical pixels.
    ///
    /// Required on those two kinds, so this is what the file says or `0` on a
    /// kind that has no such thing.
    pub fn extent(&self) -> i32 {
        self.root()
            .get("extent")
            .and_then(KdlValue::as_integer)
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(0)
    }

    /// See [`Form::title`] for what a form says about itself.
    /// The size the form was designed at, in logical pixels.
    pub fn size(&self) -> Size {
        let axis = |name: &str| {
            self.root()
                .get(name)
                .and_then(KdlValue::as_integer)
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(0)
        };
        Size::new(axis("width"), axis("height"))
    }

    /// See [`Form::title`] for what a form says about itself.
    /// The theme the file names, or the dark one.
    pub fn theme(&self) -> Theme {
        match self.theme_name() {
            "light" => theme::LIGHT,
            "high-contrast" => theme::HIGH_CONTRAST,
            _ => theme::DARK,
        }
    }

    /// See [`Form::title`] for what a form says about itself.
    /// The theme's name, as the file spells it.
    pub fn theme_name(&self) -> &str {
        self.root()
            .get("theme")
            .and_then(KdlValue::as_string)
            .unwrap_or("dark")
    }

    /// See [`Form::title`] for what a form says about itself.
    /// The surface the form is drawn on.
    pub fn background(&self) -> Role {
        self.root()
            .get("background")
            .and_then(KdlValue::as_string)
            .and_then(denise_ui::widgets::describe::role_from_name)
            .unwrap_or(Role::Base100)
    }

    /// The file as it now stands.
    ///
    /// Byte for byte what was parsed, until something edits it, and then byte for
    /// byte what was parsed **apart from what was edited**. `kdl` holds the
    /// document rather than a value taken from it, so comments, blank lines,
    /// column alignment and entry order all survive an edit to a property three
    /// nodes away. That is the round trip the designer stands on, and the reason
    /// this crate parses the way it does.
    /// ```
    /// # use denise_forms::{Edit, Form};
    /// // A comment, a blank line, and columns somebody lined up by hand.
    /// let source = "\
    /// // The panel everything sits on.
    /// form \"F\" version=1 width=320 height=240 {
    ///
    ///     label \"One\"   x=8  y=8  w=80 h=20
    ///     label \"Two\"   x=8  y=32 w=80 h=20
    /// }
    /// ";
    /// let mut form = Form::parse(source)?;
    /// assert_eq!(form.text(), source, "parsing changed nothing");
    ///
    /// // One number, one line: everything else is where it was, spacing and all.
    /// form.apply(Edit::number(&[1], "y", Some(40)))?;
    /// assert_eq!(
    ///     form.text(),
    ///     source.replace("x=8  y=32", "x=8  y=40"),
    /// );
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    pub fn text(&self) -> String {
        self.doc.to_string()
    }

    // ------------------------------------------------------------- editing

    /// The node at a child path, if there is one.
    fn at_mut(&mut self, path: &[usize]) -> Option<&mut KdlNode> {
        // The empty path is the `form` node, the same as it is for `node_at`.
        // That is what lets an edit reach the form's own properties — its size,
        // its kind, its theme — through the one door every other edit uses, and
        // so undo them the same way.
        let Some((&first, rest)) = path.split_first() else {
            return self.doc.nodes_mut().first_mut();
        };
        let mut node = self
            .doc
            .nodes_mut()
            .first_mut()?
            .children_mut()
            .as_mut()?
            .nodes_mut()
            .get_mut(first)?;
        for &index in rest {
            node = node.children_mut().as_mut()?.nodes_mut().get_mut(index)?;
        }
        Some(node)
    }

    /// Sets a whole-number property on the node at `path`.
    ///
    /// Replaces the value **in place** when the property is already there, which
    /// is what keeps a move to a one-line diff: everything else on the line, and
    /// every line around it, is untouched. Appends when it is not.
    ///
    /// Returns `false` if there is no node at that path.
    ///
    /// ```
    /// # use denise_forms::Form;
    /// let mut form = Form::parse(
    ///     "form \"F\" version=1 width=99 height=99 {\n    \
    ///      label \"hi\" x=10 y=20 w=30 h=40  // where it sits\n}\n",
    /// )?;
    /// assert!(form.set_number(&[0], "x", 25));
    /// assert!(form.text().contains("x=25 y=20"));
    /// assert!(form.text().contains("// where it sits"), "the comment survived");
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    pub fn set_number(&mut self, path: &[usize], name: &str, value: i64) -> bool {
        match self.at_mut(path) {
            // Only a `Literal::Verbatim` can fail to be written, so this is
            // always `true` when the path names a node.
            Some(node) => set_literal(node, name, &Literal::Int(value)).is_ok(),
            None => false,
        }
    }

    /// The node at `path`, or the form itself for an empty one.
    fn node_at(&self, path: &[usize]) -> Option<&KdlNode> {
        let mut node = self.doc.nodes().first()?;
        for &index in path {
            node = node.children()?.nodes().get(index)?;
        }
        Some(node)
    }

    /// What the file writes for a node's property, or `None` when it does not
    /// write it at all.
    ///
    /// The *value*, not the spelling: a string comes back unquoted, because an
    /// inspector's field edits the string and not the quotes around it. What a
    /// `None` means is the whole of "this property is at its default" — the
    /// schema does not write a default, so nothing written is the default.
    /// ```
    /// # use denise_forms::Form;
    /// let form = Form::parse(r#"form "F" version=1 width=99 height=99 { label "Hi" name=greeting x=8 y=8 w=80 h=20 }"#)?;
    ///
    /// assert_eq!(form.property(&[0], "x").as_deref(), Some("8"));
    /// // Unquoted, because a field edits the string and not the quotes.
    /// assert_eq!(form.property(&[0], "name").as_deref(), Some("greeting"));
    /// // Not written is the default.
    /// assert_eq!(form.property(&[0], "role"), None);
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    pub fn property(&self, path: &[usize], name: &str) -> Option<String> {
        Some(spell(self.node_at(path)?.get(name)?))
    }

    /// The arguments of a node's children of one kind, in file order.
    ///
    /// What a **collection** holds: a `select`'s `option`s, a `tabs`'s `tab`s, a
    /// `table`'s `column`s. Each item is the child's own argument, which is how
    /// every collection in this format writes its text.
    ///
    /// Named by the child node rather than by a plural, because that is what the
    /// file says and what [`PropertyKind::List`](denise_ui::widgets::PropertyKind::List)
    /// names: a property called `option` *is* the `option` nodes under it.
    ///
    /// ```
    /// # use denise_forms::Form;
    /// let form = Form::parse(
    ///     "form \"F\" version=1 width=99 height=99 {\n    select name=job x=0 y=0 w=9 h=9 {\n        option \"Reader\"\n        option \"Author\"\n    }\n}\n",
    /// )?;
    ///
    /// assert_eq!(form.items(&[0], "option"), ["Reader", "Author"]);
    /// // A kind the node does not hold, and a node that is not there.
    /// assert!(form.items(&[0], "tab").is_empty());
    /// assert!(form.items(&[9], "option").is_empty());
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    pub fn items(&self, path: &[usize], kind: &str) -> Vec<String> {
        let Some(node) = self.node_at(path) else {
            return Vec::new();
        };
        node.children()
            .map(|block| {
                block
                    .nodes()
                    .iter()
                    .filter(|child| child.name().value() == kind)
                    .map(|child| {
                        child
                            .entries()
                            .iter()
                            .find(|entry| entry.name().is_none())
                            .map_or_else(String::new, |entry| spell(entry.value()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// How many children a node has, of every kind.
    ///
    /// Where an appended child goes. Not the same as `items(path, kind).len()`:
    /// a `table` holds `column`s *and* `row`s, so the index among one kind is
    /// not the index among children — which is the index every edit takes.
    ///
    /// ```
    /// # use denise_forms::Form;
    /// let form = Form::parse(
    ///     "form \"F\" version=1 width=99 height=99 {\n    table name=t x=0 y=0 w=9 h=9 {\n        column \"A\"\n        row \"1\"\n        row \"2\"\n    }\n}\n",
    /// )?;
    ///
    /// assert_eq!(form.child_count(&[0]), 3);
    /// assert_eq!(form.items(&[0], "row").len(), 2);
    /// assert_eq!(form.child_count(&[9]), 0);
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    pub fn child_count(&self, path: &[usize]) -> usize {
        self.node_at(path)
            .and_then(KdlNode::children)
            .map_or(0, |block| block.nodes().len())
    }

    /// Where a node's `n`th child of one kind sits, for an edit that means it.
    ///
    /// A collection's items are addressed like any other node — see
    /// [`Edit::Argument`], [`Edit::Insert`], [`Edit::Remove`] and
    /// [`Edit::Move`], all of which already reach them — but the index among
    /// *`option`s* is not the index among children when a node holds more than
    /// one kind. This translates.
    ///
    /// ```
    /// # use denise_forms::Form;
    /// let form = Form::parse(
    ///     "form \"F\" version=1 width=99 height=99 {\n    table name=t x=0 y=0 w=9 h=9 {\n        column \"A\"\n        row \"1\"\n        row \"2\"\n    }\n}\n",
    /// )?;
    ///
    /// // The second `row` is the table's *third* child.
    /// assert_eq!(form.item_path(&[0], "row", 1), Some(vec![0, 2]));
    /// assert_eq!(form.item_path(&[0], "column", 0), Some(vec![0, 0]));
    /// // Past the end, and a kind the node does not hold.
    /// assert_eq!(form.item_path(&[0], "row", 2), None);
    /// assert_eq!(form.item_path(&[0], "option", 0), None);
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    pub fn item_path(&self, path: &[usize], kind: &str, nth: usize) -> Option<Vec<usize>> {
        let node = self.node_at(path)?;
        let at = node
            .children()?
            .nodes()
            .iter()
            .enumerate()
            .filter(|(_, child)| child.name().value() == kind)
            .map(|(index, _)| index)
            .nth(nth)?;
        let mut full = path.to_vec();
        full.push(at);
        Some(full)
    }

    /// The source of one node, as it stands in the file, with its own
    /// indentation taken off.
    ///
    /// What copying a node puts on the clipboard: `.dform` source that reads as
    /// source. Its children come with it, and so does a comment written above
    /// it — the node's leading trivia is part of the node, which is the same
    /// reason an undone removal puts the comment back.
    /// ```
    /// # use denise_forms::Form;
    /// let form = Form::parse(
    ///     "form \"F\" version=1 width=99 height=99 {\n    panel name=box x=0 y=0 w=9 h=9 {\n        label \"in\" x=1 y=1 w=2 h=2\n    }\n}\n",
    /// )?;
    ///
    /// assert_eq!(
    ///     form.node_text(&[0]).as_deref(),
    ///     Some("panel name=box x=0 y=0 w=9 h=9 {\n    label \"in\" x=1 y=1 w=2 h=2\n}\n"),
    /// );
    /// assert_eq!(form.node_text(&[9]), None);
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    pub fn node_text(&self, path: &[usize]) -> Option<String> {
        let node = self.node_at(path)?;
        let own = indent_of(node);
        Some(reindent(&node.to_string(), &own, "", false))
    }

    /// Every node in the file, depth first, the form node itself first.
    ///
    /// What can be known about a form **without building it**, which is what
    /// comparing two versions of the same file needs: [`Placed`](crate::Placed)
    /// is the same
    /// node after [`build`](Form::build) and carries a `NodeId` that only exists
    /// once there is a tree, so it cannot describe a file nobody has opened.
    ///
    /// ```
    /// # use denise_forms::Form;
    /// let form = Form::parse(
    ///     "form \"F\" version=1 width=99 height=99 {\n    panel name=box x=0 y=0 w=9 h=9 {\n        label \"in\" x=1 y=1 w=2 h=2\n    }\n}\n",
    /// )?;
    ///
    /// let written = form.written();
    /// let kinds: Vec<&str> = written.iter().map(|node| node.kind.as_str()).collect();
    /// assert_eq!(kinds, ["form", "panel", "label"]);
    /// assert_eq!(written[1].name.as_deref(), Some("box"));
    /// assert_eq!(written[2].argument.as_deref(), Some("in"));
    /// assert_eq!(written[2].path, vec![0, 0]);
    /// // The node itself, without the children indented under it.
    /// assert_eq!(written[1].line, "panel name=\"box\" x=0 y=0 w=9 h=9");
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    pub fn written(&self) -> Vec<Written> {
        let mut out = Vec::new();
        let Some(root) = self.doc.nodes().first() else {
            return out;
        };
        gather(root, &mut Vec::new(), &mut out);
        out
    }

    /// ```
    /// # use denise_forms::Form;
    /// let form = Form::parse(r#"form "F" version=1 width=99 height=99 { label "Hello" x=0 y=0 w=9 h=9 }"#)?;
    /// assert_eq!(form.argument(&[0]).as_deref(), Some("Hello"));
    /// // The form's own argument is its title.
    /// assert_eq!(form.argument(&[]).as_deref(), Some("F"));
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    /// A node's positional argument — the `"Hello"` in `label "Hello"`.
    pub fn argument(&self, path: &[usize]) -> Option<String> {
        let node = self.node_at(path)?;
        let entry = node.entries().iter().find(|e| e.name().is_none())?;
        Some(spell(entry.value()))
    }

    /// Removes a property from the node at `path`.
    ///
    /// What a designer does when a property goes back to its default: the schema
    /// says a default is not written, so resetting one is deleting it rather than
    /// spelling it out.
    /// ```
    /// # use denise_forms::Form;
    /// let mut form = Form::parse(r#"form "F" version=1 width=99 height=99 { label "Hi" x=0 y=0 w=9 h=9 role=primary }"#)?;
    ///
    /// assert!(form.clear_property(&[0], "role"));
    /// assert_eq!(form.property(&[0], "role"), None);
    /// // Nothing there to clear.
    /// assert!(!form.clear_property(&[0], "role"));
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    ///
    /// Use [`Form::apply`] with [`Edit::property`] instead where the change has
    /// to be undoable: this one hands back nothing to put it back with.
    pub fn clear_property(&mut self, path: &[usize], name: &str) -> bool {
        let Some(node) = self.at_mut(path) else {
            return false;
        };
        // Not `KdlNode::remove`, whose documentation says string keys remove
        // properties and which returns `None` and removes nothing — `entry("z")`
        // finds the property that `remove("z")` cannot. `retain` does what the
        // other was for, and leaves the rest of the line alone.
        let before = node.entries().len();
        node.retain(|entry| entry.name().map(|key| key.value()) != Some(name));
        node.entries().len() != before
    }

    /// Removes the node at `path`, and everything under it.
    ///
    /// Returns `false` if there is no node there.
    /// ```
    /// # use denise_forms::Form;
    /// let mut form = Form::parse(r#"form "F" version=1 width=99 height=99 { label "Hi" x=0 y=0 w=9 h=9 }"#)?;
    ///
    /// assert!(form.remove_at(&[0]));
    /// assert!(!form.text().contains("label"));
    /// assert!(!form.remove_at(&[0]), "there is nothing there now");
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    ///
    /// Use [`Form::apply`] with [`Edit::remove`] instead where the change has to
    /// be undoable.
    pub fn remove_at(&mut self, path: &[usize]) -> bool {
        let Some((&last, above)) = path.split_last() else {
            return false;
        };
        let Some(parent) = self.children_of_mut(above) else {
            return false;
        };
        if last >= parent.len() {
            return false;
        }
        parent.remove(last);
        true
    }

    /// Applies an edit, and hands back the edit that undoes it.
    ///
    /// The whole of undo, and the reason it is exact: because this crate holds
    /// the **document** rather than a value taken from it, the inverse of an edit
    /// is knowable at the moment it is made and is itself an ordinary edit. There
    /// is no snapshot of anything — a stack of these is a stack of small,
    /// reversible facts.
    ///
    /// ```
    /// # use denise_forms::{Edit, Form};
    /// let source = "form \"F\" version=1 width=9 height=9 {\n    \
    ///                label \"hi\" x=1 y=2 w=3 h=4  // a note\n}\n";
    /// let mut form = Form::parse(source)?;
    ///
    /// let undo = form.apply(Edit::number(&[0], "x", Some(40)))?;
    /// assert!(form.text().contains("x=40"));
    ///
    /// form.apply(undo)?;
    /// assert_eq!(form.text(), source, "byte for byte, comment and all");
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// When the path names no node, when the text of an insertion is not one
    /// node, or when a property being replaced holds something other than a
    /// whole number — which could not be put back, and so is refused rather than
    /// silently made irreversible.
    pub fn apply(&mut self, edit: Edit) -> Result<Edit, Error> {
        match edit {
            Edit::Property { path, name, value } => {
                let node = self.at_mut(&path).ok_or_else(|| {
                    Error::new(At::START, Reason::NoSuchNode { path: path.clone() })
                })?;
                let Some(literal) = value else {
                    // Taking a property away cannot be undone by putting it back
                    // by name: an added entry lands at the end of the line rather
                    // than in its place in it. The node's own text is what
                    // carries the order.
                    let text = node.to_string();
                    let key = name.clone();
                    node.retain(|entry| entry.name().map(|k| k.value()) != Some(&key));
                    return Ok(Edit::Replace { path, text });
                };

                // What was there, spelled the way the file spelled it. Anything
                // else would put `1_000` back as `1000`, which is a correct undo
                // and not an exact one.
                let before = node
                    .entry(name.as_str())
                    .map(|entry| Literal::Verbatim(repr_of(entry)));
                if let Some(was) = node.get(name.as_str()) {
                    let (holds, given) = (Class::of(was), literal.class()?);
                    if holds != given && holds != Class::Nothing {
                        return Err(Error::new(
                            At::START,
                            Reason::WrongKind {
                                name,
                                holds: holds.noun(),
                                given: given.noun(),
                            },
                        ));
                    }
                }

                set_literal(node, name.as_str(), &literal)?;
                Ok(Edit::Property {
                    path,
                    name,
                    value: before,
                })
            }

            Edit::Insert {
                parent,
                index,
                text,
            } => {
                let mut node = one_node(&text)?;
                let holder = self.node_at(&parent).ok_or_else(|| {
                    Error::new(
                        At::START,
                        Reason::NoSuchNode {
                            path: parent.clone(),
                        },
                    )
                })?;
                // A parent written without a `{ }` gets one — which is every
                // panel a designer has just placed. Undoing that has to take the
                // block away with the node, or an undone drop would leave an
                // empty pair of braces behind; the parent's own text is what
                // carries both, so the inverse replaces the parent rather than
                // removing the child.
                let empty = holder.children().is_none();
                let was = empty.then(|| holder.to_string());
                let indent = indent_of(holder);
                let step = self.indent_step();

                let bare = node.format().is_none_or(|format| format.leading.is_empty());
                let children = self.block_mut(&parent).ok_or_else(|| {
                    Error::new(
                        At::START,
                        Reason::NoSuchNode {
                            path: parent.clone(),
                        },
                    )
                })?;
                let index = index.min(children.nodes().len());

                // Trivia the caller gave stands: a removal's inverse carries the
                // node's own indentation and the comment written above it, and
                // putting that back is the whole of an exact undo. Text with
                // none — a designer's freshly seeded node — is laid out here,
                // because this is what knows how deep it is going.
                //
                // Every node ends in a newline, and its indentation is its own;
                // only the first in a block also carries the newline that
                // follows the brace, because nothing before it did.
                if bare {
                    // And *all* of it, not only its first line: a pasted panel
                    // arrives with its children, and they are as deep as the
                    // panel is now.
                    let laid = reindent(
                        &node.to_string(),
                        "",
                        &format!("{indent}{step}"),
                        index == 0,
                    );
                    node = one_node(&laid)?;
                    let mut format = node.format().cloned().unwrap_or_default();
                    format.terminator = String::from("\n");
                    node.set_format(format);
                }

                // Whether the arrival carries the newline that follows the
                // opening brace — which only the first node in a block does.
                let opens = node
                    .format()
                    .is_some_and(|format| format.leading.starts_with('\n'));
                let displaced = index == 0 && !children.nodes().is_empty();
                children.nodes_mut().insert(index, node);
                // The node that used to be first is not any more, and the line
                // it was holding open is being held by the new arrival. Leaving
                // it with its own newline would leave a blank line behind —
                // exactly the mirror of what `Remove` puts right when it takes
                // the first node out.
                if displaced
                    && opens
                    && let Some(after) = children.nodes_mut().get_mut(1)
                {
                    let mut format = after.format().cloned().unwrap_or_default();
                    if let Some(rest) = format.leading.strip_prefix('\n') {
                        format.leading = rest.to_string();
                        after.set_format(format);
                    }
                }
                if empty {
                    // The closing brace lines up under the node that opened it,
                    // and there is a space before the one that opens it.
                    let mut format = children.format().cloned().unwrap_or_default();
                    format.trailing = indent.clone();
                    children.set_format(format);
                }
                if empty && let Some(holder) = self.at_mut(&parent) {
                    let mut format = holder.format().cloned().unwrap_or_default();
                    format.before_children = String::from(" ");
                    holder.set_format(format);
                }

                match was {
                    Some(text) => Ok(Edit::Replace { path: parent, text }),
                    None => {
                        let mut path = parent;
                        path.push(index);
                        Ok(Edit::Remove { path })
                    }
                }
            }

            Edit::Argument { path, value } => {
                let node = self.at_mut(&path).ok_or_else(|| {
                    Error::new(At::START, Reason::NoSuchNode { path: path.clone() })
                })?;
                let Some(entry) = node.entries_mut().iter_mut().find(|e| e.name().is_none()) else {
                    return Err(Error::new(At::START, Reason::NoArgument));
                };
                let was = Literal::Verbatim(repr_of(entry));
                let (new, repr) = value.parts()?;
                entry.set_value(new);
                match entry.format_mut() {
                    Some(format) => format.value_repr = repr,
                    None => entry.set_format(KdlEntryFormat {
                        value_repr: repr,
                        leading: String::from(" "),
                        ..KdlEntryFormat::default()
                    }),
                }
                Ok(Edit::Argument { path, value: was })
            }

            Edit::Move { from, to, index } => {
                // A node cannot go inside itself, and the form is the document
                // rather than a node in it.
                if from.is_empty() || to.starts_with(&from) {
                    return Err(Error::new(At::START, Reason::IntoItself { path: from }));
                }
                let node = self.node_at(&from).ok_or_else(|| {
                    Error::new(At::START, Reason::NoSuchNode { path: from.clone() })
                })?;
                let text = node.to_string();
                let old = indent_of(node);

                // Where it is going, once taking it out has shifted whatever
                // came after it — the case that is easy to miss: removing `[1]`
                // moves `[2]` to `[1]`, so a move that named `[2]` as its
                // destination has to be told.
                let landing = after_removing(&to, &from).ok_or_else(|| {
                    Error::new(At::START, Reason::IntoItself { path: from.clone() })
                })?;
                let step = self.indent_step();
                let new = match self.node_at(&to) {
                    Some(parent) if !to.is_empty() => indent_of(parent) + &step,
                    Some(_) => step,
                    None => {
                        return Err(Error::new(At::START, Reason::NoSuchNode { path: to }));
                    }
                };

                // How many children the landing will have once the node has
                // left it, which decides both where the index can reach and
                // whether it becomes the first — the one position that carries
                // the newline after the brace.
                let held = self
                    .node_at(&to)
                    .and_then(KdlNode::children)
                    .map_or(0, |block| block.nodes().len());
                let leaving = from.len() == to.len() + 1 && from.starts_with(&to);
                let index = index.min(held - usize::from(leaving && held > 0));

                // The blank lines that stood above it, which travel with it.
                //
                // A node's leading newlines are **one structural** — the line
                // the brace opened, carried by whichever node is first in the
                // block — plus one for every blank line above it. Nothing in the
                // trivia says which is which; only the node's position does. So
                // the count is taken here, where the old position is still
                // known, and put back below against the new one.
                let had = text.len() - text.trim_start_matches('\n').len();
                let blanks = had.saturating_sub(usize::from(from.last() == Some(&0)));

                let text = reindent(&text, &old, &new, index == 0);
                // `reindent` leaves exactly the structural newline the landing
                // calls for; the blank lines go back on top of it. Pure newlines,
                // so the order among them does not matter — what matters is how
                // many.
                let text = if blanks == 0 {
                    text
                } else {
                    let mut with = "\n".repeat(blanks);
                    with.push_str(&text);
                    with
                };
                // Two edits as one, so the inverse is the pair reversed: put the
                // node back where it came from, with the text it had there, and
                // take away any braces this had to make. `Many` already does all
                // of that, and doing it by hand would be doing it again.
                self.apply(Edit::Many(vec![
                    Edit::Remove { path: from },
                    Edit::Insert {
                        parent: landing,
                        index,
                        text,
                    },
                ]))
            }

            Edit::Remove { path } => {
                let (&last, above) = path.split_last().ok_or_else(|| {
                    Error::new(At::START, Reason::NoSuchNode { path: Vec::new() })
                })?;
                let above = above.to_vec();
                let children = self.children_of_mut(&above).ok_or_else(|| {
                    Error::new(
                        At::START,
                        Reason::NoSuchNode {
                            path: above.clone(),
                        },
                    )
                })?;
                if last >= children.len() {
                    return Err(Error::new(At::START, Reason::NoSuchNode { path }));
                }
                // Taking the *first* node out of a block leaves the next one
                // holding the line the brace opened, and it was not written to
                // carry the newline that starts it. Putting that right changes a
                // line nobody asked about, so the inverse restores the parent's
                // whole text rather than reasoning about which newline went
                // where — the same answer this file gives every time an edit
                // cannot be undone by putting one value back.
                // Two shapes of removal cannot be undone by putting the node
                // back on its own, and both are answered the same way this file
                // answers every such case — with the parent's own text, which
                // carries the shape as well as the contents.
                //
                // Taking the *first* node out leaves the next one holding the
                // line the brace opened, and it was not written to carry the
                // newline that starts it. Taking the *last* one out leaves an
                // empty `{ }` that nobody typed.
                let opener = last == 0 && children.len() > 1;
                let emptied = children.len() == 1;
                let was = (opener || emptied).then(|| {
                    self.node_at(&above)
                        .map_or_else(String::new, |node| node.to_string())
                });

                let children = self.children_of_mut(&above).expect("just found");
                // The node's own text carries its leading trivia — its
                // indentation and any comment written above it — so putting it
                // back puts all of that back too.
                let text = children.remove(last).to_string();
                if opener && let Some(first) = children.first_mut() {
                    // Unconditionally, and that is the whole of the other half
                    // of #151. The node that is now first never held the line
                    // the brace opened, so it always needs one — and a leading
                    // newline it *already* has is a blank line somebody wrote,
                    // not the structural one. Skipping the insert when it starts
                    // with `\n` looked like idempotence and was a blank line
                    // being quietly promoted into the structural newline, so a
                    // node moved away from the front took the blank line above
                    // its neighbour with it.
                    let mut format = first.format().cloned().unwrap_or_default();
                    format.leading.insert(0, '\n');
                    first.set_format(format);
                }
                if emptied {
                    self.drop_block(&above);
                }
                match was {
                    Some(text) => Ok(Edit::Replace { path: above, text }),
                    None => Ok(Edit::Insert {
                        parent: above,
                        index: last,
                        text,
                    }),
                }
            }

            Edit::Many(edits) => {
                let mut inverses: Vec<Edit> = Vec::with_capacity(edits.len());
                for edit in edits {
                    match self.apply(edit) {
                        Ok(inverse) => inverses.push(inverse),
                        Err(error) => {
                            // Put back what was already done, newest first, so a
                            // refused compound leaves the document as it was.
                            while let Some(inverse) = inverses.pop() {
                                let _ = self.apply(inverse);
                            }
                            return Err(error);
                        }
                    }
                }
                inverses.reverse();
                Ok(Edit::Many(inverses))
            }

            Edit::Replace { path, text } => {
                let node = one_node(&text)?;
                let Some((&last, above)) = path.split_last() else {
                    // The form itself, which is the whole document: an insertion
                    // into a form written without a `{ }` undoes to this.
                    let root = self
                        .doc
                        .nodes_mut()
                        .first_mut()
                        .ok_or_else(|| Error::new(At::START, Reason::NoSuchNode { path }))?;
                    let was = root.to_string();
                    *root = node;
                    return Ok(Edit::Replace {
                        path: Vec::new(),
                        text: was,
                    });
                };
                let above = above.to_vec();
                let children = self.children_of_mut(&above).ok_or_else(|| {
                    Error::new(
                        At::START,
                        Reason::NoSuchNode {
                            path: above.clone(),
                        },
                    )
                })?;
                if last >= children.len() {
                    return Err(Error::new(At::START, Reason::NoSuchNode { path }));
                }
                let was = children[last].to_string();
                children[last] = node;
                Ok(Edit::Replace { path, text: was })
            }
        }
    }

    /// One step of indentation, as this file writes it.
    ///
    /// Read from the form's own first child rather than assumed to be four
    /// spaces, so a file written with two stays written with two.
    fn indent_step(&self) -> String {
        self.doc
            .nodes()
            .first()
            .and_then(KdlNode::children)
            .and_then(|block| block.nodes().first())
            .map(indent_of)
            .filter(|indent| !indent.is_empty())
            .unwrap_or_else(|| String::from("    "))
    }

    /// Takes away the children block of the node at `path`.
    ///
    /// So that there is no such thing as an empty one: `panel name=card` and
    /// `panel name=card { }` mean the same to the engine, and only the first is
    /// what somebody would have written.
    fn drop_block(&mut self, path: &[usize]) {
        let holder = if path.is_empty() {
            self.doc.nodes_mut().first_mut()
        } else {
            self.at_mut(path)
        };
        if let Some(node) = holder {
            *node.children_mut() = None;
        }
    }

    /// The children block of the node at `path`, made if it has none.
    fn block_mut(&mut self, path: &[usize]) -> Option<&mut KdlDocument> {
        if path.is_empty() {
            return Some(self.doc.nodes_mut().first_mut()?.ensure_children());
        }
        Some(self.at_mut(path)?.ensure_children())
    }

    /// The children of the node at `path`, or of the form itself for an empty one.
    fn children_of_mut(&mut self, path: &[usize]) -> Option<&mut Vec<KdlNode>> {
        if path.is_empty() {
            return Some(
                self.doc
                    .nodes_mut()
                    .first_mut()?
                    .children_mut()
                    .as_mut()?
                    .nodes_mut(),
            );
        }
        Some(self.at_mut(path)?.children_mut().as_mut()?.nodes_mut())
    }
}

/// Whether what follows an opening `"""` is the rest of its line and then a
/// newline, which is what makes it a multi-line string rather than an error.
fn opens_a_line(rest: &[u8]) -> bool {
    rest.iter()
        .position(|byte| !matches!(byte, b' ' | b'\t' | b'\r'))
        .is_none_or(|at| rest[at] == b'\n')
}

/// Whether a closing `"""` has only a line's own indentation in front of it,
/// which is what KDL requires of one. Anything may follow it.
fn closes_a_line(before: &[u8]) -> bool {
    before
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\t' | b'\r'))
        .is_some_and(|at| before[at] == b'\n')
}

/// What a scan of the source can refuse without parsing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unparseable {
    /// A `{` that opens one level past [`MAX_DEPTH`], and its offset.
    TooDeep(usize),
    /// A brace with no partner, and its offset. `true` for a `{` that is never
    /// closed, `false` for a `}` that closes nothing.
    Unbalanced { at: usize, open: bool },
    /// The `{` of a commented-out block that opens one level past
    /// [`MAX_COMMENTED_DEPTH`], and its offset. Commented out either way round:
    /// `/-{` on the block, or a `/-` on the node the block belongs to.
    CommentedTooDeep(usize),
}

impl Unparseable {
    /// The error to refuse the file with.
    fn error(self, source: &str) -> Error {
        match self {
            Self::TooDeep(at) => {
                Error::new(At::of(source, at), Reason::TooDeep { limit: MAX_DEPTH })
            }
            Self::Unbalanced { at, open } => {
                Error::new(At::of(source, at), Reason::Unbalanced { open })
            }
            Self::CommentedTooDeep(at) => Error::new(
                At::of(source, at),
                Reason::CommentedTooDeep {
                    limit: MAX_COMMENTED_DEPTH,
                },
            ),
        }
    }
}

/// The brace that puts the file past a limit the parser must not be asked to
/// reach, if there is one.
///
/// A scanner rather than a parse, because it has to run *before* the parser: it
/// skips comments and every shape of KDL string so that a `{` inside one is not
/// counted, and counts nothing else. Braces cannot appear in a bare identifier,
/// so what is left is structure.
///
/// **It should never recognise a string that `kdl` would not.** Every skip is a
/// stretch of bytes not counted as structure, so a string this scan believes in
/// and the parser does not is a hole through the limits below. The fuzzer found
/// three, and each one is now a condition here: a quote must close (thirty-four
/// `#` and a quote with no partner hid twenty-four slashdashes and twenty-eight
/// braces), a `"""` must open a line (one that did not hid a hundred and twenty
/// braces), and its closing `"""` must lead one.
///
/// It is not a complete agreement and cannot cheaply be made one: KDL also
/// requires every line of a multi-line string to carry the closing `"""`'s
/// indentation, and a string that breaks that rule is an error to `kdl` and a
/// string to this scan. The consequence is bounded — such a file reaches the
/// parser, which is slow on it and then refuses it — and it is the same
/// unbounded-time problem [`MAX_COMMENTED_DEPTH`] describes rather than a new
/// one. What matters is the direction of the error: being wrong the *other*
/// way, and counting the insides of something that turns out to be a string,
/// would refuse a file that parses, and none of these conditions can do that.
///
/// Two things are counted, and both are about what the parser costs rather than
/// about what a form means. Plain nesting past [`MAX_DEPTH`] overflows `kdl`'s
/// recursive descent; commented-out blocks nested past [`MAX_COMMENTED_DEPTH`]
/// send it exponential. Neither is a `Result` the parser could hand back — one
/// is a stack overflow and the other never returns — so both have to be refused
/// out here, where a byte scan is all it costs.
fn unparseable(source: &str) -> Option<Unparseable> {
    let b = source.as_bytes();
    let mut i = 0;
    let mut depth = 0usize;
    // Whether each open block was commented out, so that closing one puts the
    // count back, and how many of them are open right now.
    let mut blocks: Vec<bool> = Vec::new();
    // Where each open block's `{` is, for an error that points at the brace
    // rather than at the end of the file.
    let mut opened: Vec<usize> = Vec::new();
    let mut commented = 0usize;
    // Set by a `/-` and cleared at the end of the node it commented out, so a
    // `{` reached while it is set is that node's own children block.
    let mut next_commented = false;

    while i < b.len() {
        let before = i;
        match b[i] {
            // A line comment.
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            // A block comment, which nests.
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let mut open = 1usize;
                i += 2;
                while i < b.len() && open > 0 {
                    if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                        open += 1;
                        i += 2;
                    } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                        open -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            }
            // A raw string, `#"..."#` with matching hashes. Bare `#true` and
            // friends fall through harmlessly: no quote follows the hashes.
            b'#' => {
                let mut hashes = 0usize;
                while b.get(i) == Some(&b'#') {
                    hashes += 1;
                    i += 1;
                }
                if b.get(i) == Some(&b'"') {
                    let mut at = i + 1;
                    let mut end = None;
                    while at < b.len() {
                        if b[at] == b'"' {
                            let mut past = at + 1;
                            let mut seen = 0usize;
                            while seen < hashes && b.get(past) == Some(&b'#') {
                                past += 1;
                                seen += 1;
                            }
                            if seen == hashes {
                                end = Some(past);
                                break;
                            }
                        }
                        at += 1;
                    }
                    // A run of hashes with no matching close is not a raw
                    // string, so the scan must not swallow the rest of the file
                    // on the strength of it.
                    i = end.unwrap_or(i);
                }
            }
            // A multi-line string, which KDL spells `"""` *and a newline*. A
            // `"""` with anything else after it is a parse error to kdl rather
            // than a string, so treating it as one here would skip over
            // structure kdl is going to read.
            b'"' if b[i..].starts_with(b"\"\"\"") && opens_a_line(&b[i + 3..]) => {
                // And it ends at the first `\"\"\"` that a line's own whitespace
                // leads up to. Not simply the next one anywhere: `hi\"\"\"` on the
                // end of a line does not close a multi-line string, and reading
                // it as one would skip whatever came after -- five braces, in
                // the input that found this.
                let mut at = i + 3;
                let mut end = None;
                while at + 3 <= b.len() {
                    if b[at..].starts_with(b"\"\"\"") && closes_a_line(&b[i + 3..at]) {
                        end = Some(at + 3);
                        break;
                    }
                    at += 1;
                }
                i = end.unwrap_or(i + 1);
            }
            b'"' => {
                let mut at = i + 1;
                while at < b.len() && b[at] != b'"' {
                    // An escaped character, including an escaped quote.
                    at += if b[at] == b'\\' { 2 } else { 1 };
                }
                // `at` can overshoot on a trailing backslash, which is one
                // more way for a quote not to close.
                i = if at < b.len() { at + 1 } else { i + 1 };
            }
            // A slashdash. It comments out the node that follows it, and when
            // that node carries a children block with anything in it, `kdl`
            // pays for the block twice over at every level of nesting.
            b'/' if b.get(i + 1) == Some(&b'-') => {
                next_commented = true;
                i += 2;
            }
            b'{' => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Some(Unparseable::TooDeep(i));
                }
                blocks.push(next_commented);
                opened.push(i);
                if next_commented {
                    commented += 1;
                    if commented > MAX_COMMENTED_DEPTH {
                        return Some(Unparseable::CommentedTooDeep(i));
                    }
                }
                next_commented = false;
                i += 1;
            }
            b'}' => {
                // A `}` with nothing open cannot be parsed by anything, and
                // saying so here costs one byte of lookback.
                let Some(was_commented) = blocks.pop() else {
                    return Some(Unparseable::Unbalanced { at: i, open: false });
                };
                opened.pop();
                depth -= 1;
                if was_commented {
                    commented -= 1;
                }
                i += 1;
            }
            _ => i += 1,
        }
        // A node ends at a newline or a `;`, and so does the reach of a
        // slashdash waiting for a block. Reading it off the bytes just skipped
        // covers the newline in the open and the one inside a block comment or
        // a multi-line string alike.
        //
        // `min` because an unterminated string leaves `i` past the end — the
        // arms above step over the closing quote whether or not it was there,
        // which the loop condition forgives and a slice would not.
        let skipped = &b[before..i.min(b.len())];
        if next_commented && skipped.iter().any(|byte| *byte == b'\n' || *byte == b';') {
            next_commented = false;
        }
    }
    // A `{` still open at the end of the file is the other half of the same
    // thing. `opened` holds where each one started, so the error points at the
    // brace that was never closed rather than at the end of the file.
    opened.last().map(|at| Unparseable::Unbalanced {
        at: *at,
        open: true,
    })
}

/// Parses form-file text that must be exactly one node.
fn one_node(text: &str) -> Result<KdlNode, Error> {
    let doc: KdlDocument = text.parse().map_err(|error: kdl::KdlError| {
        let message = error
            .diagnostics
            .first()
            .and_then(|d| d.message.clone())
            .unwrap_or_else(|| String::from("this is not a node"));
        Error::new(At::START, Reason::Syntax(message))
    })?;
    match doc.nodes() {
        [only] => Ok(only.clone()),
        other => Err(Error::new(
            At::START,
            Reason::Syntax(format!("this must be one node; it is {}", other.len())),
        )),
    }
}

/// The top-level nodes of a fragment of form source, each as its own text.
///
/// A *fragment* is what the nodes of a form look like without the `form` node
/// around them: what copying a selection produces, and what somebody who typed
/// a widget into a text editor is likely to have. Each node comes back with its
/// own indentation taken off, ready for [`Edit::Insert`] to lay it out wherever
/// it is going.
///
/// Every `name=` in it that `taken` already holds is given a number until it
/// does not collide, and every name it settles on is added to `taken`. So
/// pasting the same fragment twice gives two sets of names rather than two
/// clashes, and the caller passes in the names the document already uses.
///
/// This answers whether the text is *nodes*. Whether they are widgets this
/// engine knows, with properties it has, is what [`Form::build`] answers — the
/// schema lives with the builder and there is no second copy of it here, so a
/// caller that cares builds the fragment before pasting it.
///
/// ```
/// # use denise_forms::fragment;
/// let mut taken = vec![String::from("card")];
/// let nodes = fragment("panel name=card x=0 y=0 w=10 h=10", &mut taken)?;
/// assert_eq!(nodes, vec![String::from("panel name=card2 x=0 y=0 w=10 h=10")]);
/// assert_eq!(taken, vec![String::from("card"), String::from("card2")]);
/// # Ok::<(), denise_forms::Error>(())
/// ```
pub fn fragment(text: &str, taken: &mut Vec<String>) -> Result<Vec<String>, Error> {
    // The same two guards a whole file gets, and for the same reason: this text
    // came from the system clipboard, which is to say from anywhere.
    if text.len() > MAX_SOURCE {
        return Err(Error::new(
            At::START,
            Reason::TooLarge { limit: MAX_SOURCE },
        ));
    }
    if let Some(refusal) = unparseable(text) {
        return Err(refusal.error(text));
    }

    let mut doc: KdlDocument = text.parse().map_err(|error: kdl::KdlError| {
        let first = error.diagnostics.first();
        let at = first.map_or(At::START, |d| At::of(text, d.span.offset()));
        let message = first
            .and_then(|d| d.message.clone())
            .unwrap_or_else(|| String::from("this is not form source"));
        Error::new(at, Reason::Syntax(message))
    })?;

    for node in doc.nodes_mut() {
        rename_apart(node, taken)?;
    }
    Ok(doc
        .nodes()
        .iter()
        .map(|node| reindent(&node.to_string(), &indent_of(node), "", false))
        .collect())
}

/// Gives every `name=` in a subtree one that `taken` does not already hold.
fn rename_apart(node: &mut KdlNode, taken: &mut Vec<String>) -> Result<(), Error> {
    if let Some(entry) = node.entry("name") {
        let was = spell(entry.value());
        let now = unused(&was, taken);
        if now != was {
            set_literal(node, "name", &Literal::Name(now.clone()))?;
        }
        taken.push(now);
    }
    if let Some(block) = node.children_mut() {
        for child in block.nodes_mut() {
            rename_apart(child, taken)?;
        }
    }
    Ok(())
}

/// `name` if nobody has it, and `name2`, `name3`… until somebody does not.
///
/// A name that already ends in digits carries on from its stem, so pasting
/// `nav2` gives `nav3` rather than `nav22`.
fn unused(name: &str, taken: &[String]) -> String {
    if !taken.iter().any(|held| held == name) {
        return String::from(name);
    }
    let stem = name.trim_end_matches(|c: char| c.is_ascii_digit());
    let stem = if stem.is_empty() { name } else { stem };
    (2usize..)
        .map(|number| format!("{stem}{number}"))
        .find(|candidate| !taken.iter().any(|held| held == candidate))
        .unwrap_or_else(|| String::from(name))
}

/// A path as it stands once `removed` has been taken out.
///
/// `None` when the path was inside what was removed. The interesting case is the
/// quiet one: taking node `[1]` out moves `[3]` to `[2]`, so anything holding a
/// path across a removal has to be told — a move that names a destination
/// *after* its source, and an editor holding a selection.
///
/// ```
/// # use denise_forms::after_removing;
/// // The node after the one taken out slides up.
/// assert_eq!(after_removing(&[3], &[1]), Some(vec![2]));
/// // One before it does not, and neither does one in another parent.
/// assert_eq!(after_removing(&[0], &[1]), Some(vec![0]));
/// assert_eq!(after_removing(&[5, 3], &[1]), Some(vec![4, 3]));
/// // And a path inside what was removed is nowhere at all.
/// assert_eq!(after_removing(&[1, 0], &[1]), None);
/// ```
pub fn after_removing(path: &[usize], removed: &[usize]) -> Option<Vec<usize>> {
    if path.starts_with(removed) {
        return None;
    }
    let (index, ancestors) = removed.split_last()?;
    let mut out = path.to_vec();
    if out.len() > ancestors.len() && out.starts_with(ancestors) && out[ancestors.len()] > *index {
        out[ancestors.len()] -= 1;
    }
    Some(out)
}

/// A node's text, moved from one depth to another.
///
/// Every line that begins with the old indentation gets the new one instead, so
/// the node's children move with it and keep their shape relative to it. `first`
/// says whether it is becoming the first node in its block, which is the one
/// position that also carries the newline after the brace.
fn reindent(text: &str, old: &str, new: &str, first: bool) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for (index, line) in text
        .trim_start_matches('\n')
        .split_inclusive('\n')
        .enumerate()
    {
        if old.is_empty() {
            // Nothing to swap: indent what is there rather than every blank line.
            if !line.trim().is_empty() {
                out.push_str(new);
            }
            out.push_str(line);
            continue;
        }
        match line.strip_prefix(old) {
            Some(rest) => {
                out.push_str(new);
                out.push_str(rest);
            }
            // A line shallower than the node itself can only be part of its
            // leading trivia, and a blank line has nothing to indent.
            None if index == 0 => {
                out.push_str(new);
                out.push_str(line.trim_start());
            }
            None => out.push_str(line),
        }
    }
    // Where it is going decides this, not where it came from: only the first
    // node in a block carries the newline that follows the brace, and a node
    // that was first and is landing third would otherwise leave a blank line
    // behind it.
    if first {
        out.insert(0, '\n');
    }
    out
}

/// The whitespace a node is written after, on its own line.
///
/// What an inserted child is indented one step past. Read from the file rather
/// than counted from the depth, so a form written with two spaces stays written
/// with two spaces.
/// Walks a node and everything under it into [`Written`]s, depth first.
fn gather(node: &KdlNode, path: &mut Vec<usize>, out: &mut Vec<Written>) {
    let mut line = String::from(node.name().value());
    for entry in node.entries() {
        line.push(' ');
        line.push_str(&shown(entry));
    }
    out.push(Written {
        path: path.clone(),
        kind: node.name().value().to_string(),
        name: node.get("name").map(spell),
        argument: node
            .entries()
            .iter()
            .find(|entry| entry.name().is_none())
            .map(|entry| spell(entry.value())),
        line,
    });
    let Some(children) = node.children() else {
        return;
    };
    for (index, child) in children.nodes().iter().enumerate() {
        path.push(index);
        gather(child, path, out);
        path.pop();
    }
}

/// One entry as a form file would write it, with none of the file's own spacing.
fn shown(entry: &KdlEntry) -> String {
    let value = match entry.value().as_string() {
        Some(text) => quoted(text),
        None => entry.value().to_string(),
    };
    match entry.name() {
        Some(name) => format!("{}={value}", name.value()),
        None => value,
    }
}

fn indent_of(node: &KdlNode) -> String {
    let leading = node.format().map_or("", |format| format.leading.as_str());
    let line = leading.rsplit('\n').next().unwrap_or("");
    line.chars().filter(|c| c.is_whitespace()).collect()
}

/// Parses form-file text that must be exactly one value.
///
/// What checks a [`Literal::Verbatim`] before it is written: the text goes into
/// the file as it stands, so it has to be one value and not, say, `1 x=2`.
fn one_value(text: &str) -> Result<KdlValue, Error> {
    let entry = KdlEntry::parse(text).map_err(|error: kdl::KdlError| {
        let message = error
            .diagnostics
            .first()
            .and_then(|d| d.message.clone())
            .unwrap_or_else(|| String::from("this is not a value"));
        Error::new(At::START, Reason::Syntax(message))
    })?;
    if entry.name().is_some() {
        return Err(Error::new(
            At::START,
            Reason::Syntax(String::from("this must be a value, not a property")),
        ));
    }
    Ok(entry.value().clone())
}

/// The text a property's value was written with.
///
/// An entry that came from a parse remembers it. One built in memory does not,
/// and renders canonically — which is then what it was written with, since
/// nobody has written it yet.
fn repr_of(entry: &KdlEntry) -> String {
    entry.format().map_or_else(
        || entry.value().to_string(),
        |format| format.value_repr.clone(),
    )
}

/// A value as an inspector's field should show it.
///
/// A string is its own text with nothing around it; everything else is what the
/// file would write.
fn spell(value: &KdlValue) -> String {
    match value.as_string() {
        Some(text) => text.to_string(),
        None => value.to_string(),
    }
}

/// A string as a form file would quote it.
///
/// [`KdlValue`]'s own rendering writes a plain identifier bare, which is right
/// for [`Literal::Name`] and wrong for [`Literal::Text`]: a label whose text
/// happens to be one word is still a string, and `text=Save` in a file whose
/// every other string is quoted reads as a mistake.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '\\' | '"' => {
                out.push('\\');
                out.push(character);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Sets a property, keeping everything about the line but the value.
///
/// Two things here are not what the obvious code would do, and both were found by
/// the test that asserts an edit undone is byte-for-byte what it was.
///
/// `KdlNode::insert` on a property that is already there **replaces the entry**,
/// and the replacement carries default spacing — so a line whose properties were
/// deliberately lined up in columns loses that the first time anything is
/// dragged. Reaching for the existing entry keeps its leading whitespace.
///
/// And `KdlEntry::set_value` alone is a **silent no-op** for anything that came
/// from a parse: the entry keeps the text it was written with, and renders that
/// rather than the value it now holds. The cached representation has to be set
/// too, which is why this is a function and not a line.
fn set_literal(node: &mut KdlNode, name: &str, literal: &Literal) -> Result<(), Error> {
    let (value, repr) = literal.parts()?;
    if let Some(entry) = node.entry_mut(name) {
        entry.set_value(value);
        match entry.format_mut() {
            Some(format) => format.value_repr = repr,
            // Built in memory rather than parsed, so there is no spacing to
            // keep and the whole format is this crate's to write.
            None => entry.set_format(KdlEntryFormat {
                value_repr: repr,
                leading: String::from(" "),
                ..KdlEntryFormat::default()
            }),
        }
        return Ok(());
    }
    // Nothing there to keep the shape of; appending is right.
    let mut entry = KdlEntry::new_prop(name, value);
    entry.set_format(KdlEntryFormat {
        value_repr: repr,
        leading: String::from(" "),
        ..KdlEntryFormat::default()
    });
    node.push(entry);
    Ok(())
}

/// Puts back the bytes kdl eats after a closing brace.
///
/// `}  \n` parses and serialises back as `}` — the spaces *and* the newline
/// both gone, so the next node lands on the brace's line; `}  // a note` and
/// `} /* a note */` lose the comment the same way. All found by the fuzz
/// target `parse_form` within its first hour. A plain node's terminator keeps
/// its trivia, but whatever stands between a `}` and the next node is consumed
/// into a terminator that is then stored empty.
///
/// The bytes are recoverable because kdl still records where every node
/// *starts*, and its leading trivia with it. So the rule is a simple one, and
/// it is the same rule for a loss and for a file with nothing wrong: a node's
/// terminator is every byte between the end of what it renders as and the
/// beginning of what the next node owns. Applying it to a file kdl kept
/// intact reproduces the terminator kdl already stored.
///
/// Children first, because a repaired child grows its parent's rendering and
/// the end of that rendering is what the arithmetic measures from.
///
/// `Form::parse` verifies the whole document against the source afterwards, so
/// a shape this gets wrong is refused there rather than saved back corrupted.
fn restore_after_close(doc: &mut KdlDocument, source: &str) {
    for node in doc.nodes_mut() {
        restore_subtree(node, source);
    }
    // The document's own trailing trivia is kept as it is; the last node runs
    // up to where that begins.
    let trailing = doc.format().map_or(0, |format| format.trailing.len());
    let Some(limit) = source.len().checked_sub(trailing) else {
        return;
    };
    terminate_block(doc, limit, source);
}

/// Repairs the children of `node`, and theirs, but not `node`'s own
/// terminator — that belongs to whoever owns the block `node` sits in.
///
/// See [`restore_after_close`].
fn restore_subtree(node: &mut KdlNode, source: &str) {
    let Some(block) = node.children_mut() else {
        return;
    };
    if block.nodes().is_empty() {
        // Nothing after `{` to bound, and an empty block keeps its own bytes;
        // anything lost after the `}` is this node's terminator, one level up.
        return;
    }
    for child in block.nodes_mut() {
        restore_subtree(child, source);
    }
    // Only now does the last child render in full, so only now is the end of
    // it the true byte offset that the walk to the closing brace starts from.
    let trailing = block
        .format()
        .map_or_else(String::new, |format| format.trailing.clone());
    let last = block.nodes().last().expect("the block is not empty");
    let Some(limit) = end_of_nodes(source, content_end(last), &trailing) else {
        return;
    };
    terminate_block(block, limit, source);
}

/// Gives every node in one block the bytes that stand between it and the next.
fn terminate_block(block: &mut KdlDocument, limit: usize, source: &str) {
    let bounds: Vec<usize> = (0..block.nodes().len())
        .map(|i| block.nodes().get(i + 1).map_or(limit, owned_start))
        .collect();
    for (node, bound) in block.nodes_mut().iter_mut().zip(bounds) {
        let from = content_end(node);
        // A bound below the node's end, or one that lands inside a character,
        // means the arithmetic missed; leave the node alone and let the verify
        // in `Form::parse` refuse the file.
        let Some(terminator) = source.get(from..bound) else {
            continue;
        };
        let format = node.format().cloned().unwrap_or_default();
        if terminator != format.terminator {
            node.set_format(KdlNodeFormat {
                terminator: terminator.to_string(),
                ..format
            });
        }
    }
}

/// The first byte of a node — its leading trivia, not its name.
fn owned_start(node: &KdlNode) -> usize {
    let leading = node.format().map_or(0, |format| format.leading.len());
    node.span().offset().saturating_sub(leading)
}

/// The byte just past what a node renders as, which for a node with children
/// is the byte just past its `}`.
fn content_end(node: &KdlNode) -> usize {
    let format = node.format().cloned().unwrap_or_default();
    let rendered = node.to_string().len();
    let inner = rendered.saturating_sub(format.leading.len() + format.terminator.len());
    node.span().offset() + inner
}

/// Where a block's nodes stop and the block's own trailing trivia begins.
///
/// Walking from `from` — the end of the block's last node — to the closing
/// brace crosses only trivia, and the block keeps the tail of it in `trailing`
/// already. So the answer is the first point at which `trailing` and then `}`
/// stand next in the source. Testing that before each step of the walk rather
/// than after it is what lets `trailing` start with whitespace of its own.
///
/// `None` when the walk meets something it does not recognise, which leaves
/// the block untouched and the file refused if kdl did lose bytes there.
fn end_of_nodes(source: &str, from: usize, trailing: &str) -> Option<usize> {
    let mut at = from;
    loop {
        let rest = source.get(at..)?;
        if rest
            .strip_prefix(trailing)
            .is_some_and(|past| past.starts_with('}'))
        {
            return Some(at);
        }
        at += trivia_width(rest)?;
    }
}

/// The length of the one piece of trivia at the front of `rest`, or `None` if
/// what stands there is not trivia. A slashdash never reaches this: kdl keeps
/// commented-out nodes in the block's `trailing`, so the walk stops at the `/`
/// and the match against `trailing` has already succeeded there.
fn trivia_width(rest: &str) -> Option<usize> {
    let first = rest.chars().next()?;
    if first.is_whitespace() {
        return Some(first.len_utf8());
    }
    if let Some(body) = rest.strip_prefix("//") {
        return Some(2 + body.find('\n').map_or(body.len(), |end| end + 1));
    }
    if !rest.starts_with("/*") {
        return None;
    }
    // KDL's block comments nest, so this counts rather than searching for the
    // first `*/`. Every delimiter is ASCII, so the returned width lands on a
    // character boundary however the comment is spelled.
    let bytes = rest.as_bytes();
    let mut depth = 0usize;
    let mut at = 0usize;
    while at + 1 < bytes.len() {
        match &bytes[at..at + 2] {
            b"/*" => {
                depth += 1;
                at += 2;
            }
            b"*/" => {
                depth -= 1;
                at += 2;
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => at += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses as kdl does, repairs, and hands back what would be saved.
    fn reproduced(source: &str) -> String {
        let mut doc: KdlDocument = source.parse().expect("the shape under test parses");
        restore_after_close(&mut doc, source);
        doc.to_string()
    }

    #[test]
    fn a_brace_keeps_what_follows_it_to_the_end_of_the_line() {
        // Every one of these loses bytes in kdl itself. The first was found by
        // hand, the rest by the fuzz target `parse_form`.
        for source in [
            // Whitespace after a closing brace, then a sibling.
            "a {\n  b 1\n}  \nc 3\n",
            // A line comment in the same place.
            "a {\n  b 1\n}  // x\nc 3\n",
            // And a block comment, which may span lines of its own.
            "a {\n  b 1\n} /* p\nq */\nc 3\n",
            // The brace ends the file, with and without a final newline.
            "a {\n  b 1\n}  // x\n",
            "a {\n  b 1\n}  // x",
            // A blank line after the comment must survive as a blank line.
            "a {\n  b 1\n}  // x\n\nc 3\n",
            // The lossy brace is the last node of a block, so the walk to the
            // outer `}` is what finds the bound.
            "o {\n  a {\n    b 1\n  } // x\n}\n",
            "o {\n  a {\n    b 1\n  } /* p\nq */\n}\n",
            // The block's own trailing trivia stands between the two, both as
            // indentation and as a commented-out node.
            "a {\n  b {\n    c 1\n  }  // x\n}\n",
            "a {\n  b {\n    c 1\n  } /-d 2\n}\n",
            // A brace inside the comment is not the brace being looked for.
            "a {\n  b 1\n} // closes }\nc 3\n",
            // Nested block comments, which KDL counts rather than terminating
            // at the first `*/`.
            "a {\n  b 1\n} /* p /* q */ r */\nc 3\n",
            // An empty block, whose loss is the node's own terminator.
            "a {}  // x\nc 3\n",
        ] {
            assert_eq!(reproduced(source), source, "in {source:?}");
        }
    }

    /// A shape the repair cannot reach, and the refusal that covers it.
    ///
    /// `kdl` records `before_ty_name`, `after_ty_name` and `after_ty` when it
    /// reads a node's type annotation and then writes none of them, so `(Z) h`
    /// comes back as `(Z)h`. Nothing this crate can set to a `KdlNodeFormat`
    /// changes that, which is what `Reason::NotPreserved` is *for*: the file is
    /// refused rather than accepted and corrupted on the first save. Found by
    /// the fuzz target `parse_form` in six bytes.
    ///
    /// If kdl ever writes those fields, this test fails and the refusal can go.
    #[test]
    fn a_type_annotation_kdl_cannot_write_back_is_refused_rather_than_mangled() {
        for source in ["(Z) h", "( Z )h", "(Z) h\n", "(Z)h { (Y) i }\n"] {
            let doc: KdlDocument = source.parse().expect("kdl reads it");
            let mut repaired = doc;
            restore_after_close(&mut repaired, source);
            assert_ne!(
                repaired.to_string(),
                source,
                "kdl now keeps {source:?} -- the refusal below can go"
            );
        }
        // And the door this crate actually puts in front of that.
        let error = Form::parse("(Z) h").expect_err("cannot be reproduced");
        assert!(matches!(error.reason, Reason::NotPreserved), "{error}");
        // The type annotation itself is fine when nothing is lost around it.
        let kept = "(Z)h\n";
        let doc: KdlDocument = kept.parse().expect("kdl reads it");
        assert_eq!(doc.to_string(), kept);
    }

    #[test]
    fn a_file_kdl_keeps_intact_is_left_exactly_as_it_was() {
        // The repair runs over every file, so the shapes with nothing wrong
        // matter as much as the shapes with something wrong.
        for source in [
            "a {\n  b 1\n}\nc 3\n",
            "a 1; b 2\n",
            "a { b 1 }; c 2\n",
            "a \\\n  1\nc 3\n",
            "a {\n  b 1\n  /-c 2\n}\n",
            "a {\n}\nc 3\n",
            "// a leading comment\na 1\n",
            "a 1\n// and a trailing one\n",
            "\n\na 1\n\n\nb 2\n",
            "a \"a string with } and // in it\"\n",
        ] {
            assert_eq!(reproduced(source), source, "in {source:?}");
        }
    }

    #[test]
    fn nesting_within_the_limit_is_allowed() {
        let source = "a ".to_string() + &"{ b ".repeat(MAX_DEPTH) + &"}".repeat(MAX_DEPTH);
        assert_eq!(unparseable(&source), None);
    }

    #[test]
    fn one_level_past_the_limit_is_caught_before_the_parser_sees_it() {
        let deep = MAX_DEPTH + 1;
        let source = "a ".to_string() + &"{ b ".repeat(deep) + &"}".repeat(deep);
        assert!(matches!(
            unparseable(&source),
            Some(Unparseable::TooDeep(_))
        ));
    }

    #[test]
    fn commented_out_blocks_are_allowed_until_they_nest() {
        // One is a person taking a widget and its children out for a minute.
        for levels in 0..=MAX_COMMENTED_DEPTH {
            let source = "a /-{ ".repeat(levels) + &"}".repeat(levels);
            assert_eq!(unparseable(&source), None, "at {levels} levels");
        }
        // Side by side is not nesting, however many there are: the cost is the
        // nesting, so the limit is on the nesting.
        let side_by_side = "a /-{ }\n".repeat(32);
        assert_eq!(unparseable(&side_by_side), None);
        // A slashdash on a node that carries no block is not counted at all,
        // which is the shape a person actually writes -- one widget taken out.
        let plain = "/- label \"x\" y=1\n".repeat(32);
        assert_eq!(unparseable(&plain), None);
        // Nor is a slashdash whose node ends before any block begins: the `{`
        // on the next line belongs to the node after it.
        let separated = "/- a\nb {\n}\n".repeat(16);
        assert_eq!(unparseable(&separated), None);
        assert_eq!(unparseable(&"/- a; b {\n}\n".repeat(16)), None);
        // And a `{` inside a string after a slashdash is not a block.
        let quoted = "/- a \"{{{{\"\n".repeat(16);
        assert_eq!(unparseable(&quoted), None);
    }

    #[test]
    fn commented_out_blocks_nested_past_the_limit_never_reach_the_parser() {
        // Twenty of these is twenty seconds inside kdl, and the sixty-four
        // MAX_DEPTH would otherwise allow does not finish at all. The scan that
        // refuses them costs one pass over a hundred bytes.
        let deep = MAX_COMMENTED_DEPTH + 1;
        let source = "a /-{ ".repeat(deep) + &"}".repeat(deep);
        assert!(matches!(
            unparseable(&source),
            Some(Unparseable::CommentedTooDeep(_))
        ));
        // Unclosed, spaced out, and with the whole file around it, the same.
        assert!(matches!(
            unparseable(&"a /-  {\n".repeat(deep)),
            Some(Unparseable::CommentedTooDeep(_))
        ));
        // And the shape the fuzzer actually found: the slashdash is on the
        // node, the block is the node's own, and there is something in it.
        assert!(matches!(
            unparseable(&"/- a b c {\n  d 1\n".repeat(deep)),
            Some(Unparseable::CommentedTooDeep(_))
        ));
    }

    #[test]
    fn a_triple_quote_is_only_a_string_when_it_opens_a_line() {
        // KDL spells a multi-line string `"""` and then a newline. kdl refuses
        // `""" x`, so a scan that read it as a string would skip whatever came
        // next -- which is how one fuzzed input hid a hundred and twenty braces.
        let hidden = String::from("a x=\"\"\" y\n") + &"b {\n".repeat(MAX_DEPTH + 1);
        assert!(
            matches!(unparseable(&hidden), Some(Unparseable::TooDeep(_))),
            "a `\"\"\"` that opens no line must hide nothing"
        );
        // Nor does a `\"\"\"` on the end of a line close one, so what follows
        // that line is structure and gets counted.
        let closed_wrong = String::from("a x=\"\"\"\nhi\"\"\"\n") + &"b {\n".repeat(MAX_DEPTH + 1);
        assert!(
            matches!(unparseable(&closed_wrong), Some(Unparseable::TooDeep(_))),
            "a `\"\"\"` that closes no line must hide nothing"
        );
        // The real thing still hides what is inside it, newline and all.
        let real = "a x=\"\"\"\n{{{{{{{{\n\"\"\"\nb 1\n";
        assert_eq!(unparseable(real), None);
        // Trailing spaces before the newline are still opening a line, and
        // indentation in front of the closer is still closing one.
        let padded = "a x=\"\"\"   \n{{{{{{{{\n   \"\"\"\nb 1\n";
        assert_eq!(unparseable(padded), None);
        // Something after the closing `\"\"\"` is allowed and changes nothing.
        let after = "a x=\"\"\"\n{{{{{{{{\n\"\"\" y=2\nb 1\n";
        assert_eq!(unparseable(after), None);
    }

    #[test]
    fn a_brace_with_no_partner_is_refused_before_the_parser_looks_for_one() {
        // Neither of these can parse however long kdl spends deciding so, and
        // kdl can spend an unbounded amount of time on exactly this shape --
        // every slow input the fuzzer has found is wildly unbalanced.
        assert!(matches!(
            unparseable("a {\n  b 1\n"),
            Some(Unparseable::Unbalanced { open: true, .. })
        ));
        assert!(matches!(
            unparseable("a 1\n}\n"),
            Some(Unparseable::Unbalanced { open: false, .. })
        ));
        // The position is the brace itself, not the end of the file.
        let Some(Unparseable::Unbalanced { at, open: true }) = unparseable("a {\n  b {\n  }\n")
        else {
            panic!("the outer brace is never closed")
        };
        assert_eq!(at, 2, "the outer `{{`, not the inner one");
        // Balanced is balanced, however it is spelled.
        for source in [
            "a { }",
            "a {\n}\n",
            "a { b { c { } } }",
            "a 1",
            "",
            "// nothing\n",
        ] {
            assert_eq!(unparseable(source), None, "in {source:?}");
        }
    }

    #[test]
    fn a_brace_inside_a_string_is_not_structure() {
        for source in [
            r#"a "{{{{{{{{{{{{{{{{" b"#,
            r##"a #"{{{{{{{{{{{{{{{{"# b"##,
            "a \"\"\"\n{{{{{{{{{{{{{{{{\n\"\"\" b",
            "a // {{{{{{{{{{{{{{{{{{{{{{{{{{{{\n b",
            "a /* {{{{{{{{{{{{{{{{{{{{{{{{{{ */ b",
        ] {
            assert_eq!(unparseable(source), None, "in {source}");
        }
    }

    #[test]
    fn an_unterminated_string_does_not_loop_forever() {
        assert_eq!(unparseable("a \"unterminated"), None);
        assert_eq!(unparseable("a #\"unterminated"), None);
        assert_eq!(unparseable("a /* unterminated"), None);
        // Stepping over a closing quote that is not there leaves the scan past
        // the end of the source, which only a slashdash makes visible: it is
        // what asks for the bytes just read. Eighteen of them used to panic.
        assert_eq!(unparseable("/- a \"unterminated"), None);
        assert_eq!(unparseable("/- a #\"unterminated"), None);
        assert_eq!(unparseable("/- a /* unterminated"), None);
        assert_eq!(unparseable("/- \"x\\"), None);
        assert_eq!(unparseable("/- a \"\"\"unterminated"), None);
    }

    #[test]
    fn a_quote_that_never_closes_hides_nothing_behind_it() {
        // Every one of these opens a string the file never closes, and every
        // one of them used to blind the scan to the whole rest of the file --
        // which is how a fuzzed input walked twenty-four slashdashes and
        // twenty-eight braces past both limits. kdl does not stop reading at an
        // unclosed quote either, so neither may this.
        for opener in ["\"", "###############\"", "\"\"\"", "#\""] {
            // (a bare `"""` closes nothing and opens no line, so it hides
            // nothing either way round)
            let hidden = format!("a {opener}\n") + &"b /-{ ".repeat(8);
            assert!(
                matches!(unparseable(&hidden), Some(Unparseable::CommentedTooDeep(_))),
                "behind {opener:?}"
            );
            let deep = format!("a {opener}\n") + &"{ b ".repeat(MAX_DEPTH + 1);
            assert!(
                matches!(unparseable(&deep), Some(Unparseable::TooDeep(_))),
                "behind {opener:?}"
            );
        }
        // A string that *does* close still hides what is inside it.
        let closed = String::from("a \"{ { { { \"\n") + &"b /-{ ".repeat(8);
        assert!(matches!(
            unparseable(&closed),
            Some(Unparseable::CommentedTooDeep(_))
        ));
        assert_eq!(unparseable("a \"{ { { { { { { { \" b"), None);
    }
}
