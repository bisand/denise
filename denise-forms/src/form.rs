//! The file, parsed but not yet built.

use denise::{Role, Size, Theme, theme};
use kdl::{KdlDocument, KdlNode, KdlValue};

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

/// The themes a form may name.
const THEMES: &[&str] = &["dark", "light", "high-contrast"];

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
        if let Some(at) = too_deep(source) {
            return Err(Error::new(
                At::of(source, at),
                Reason::TooDeep { limit: MAX_DEPTH },
            ));
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

        self.named(root, "kind", FormKind::NAMES, FormKind::from_name)?;
        self.named(root, "theme", THEMES, |n| THEMES.contains(&n).then_some(()))?;
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

    /// The form's title — a window's title bar, and the designer's name for it.
    pub fn title(&self) -> &str {
        self.root()
            .entries()
            .iter()
            .find(|e| e.name().is_none())
            .and_then(|e| e.value().as_string())
            .unwrap_or_default()
    }

    /// The form's identifier, if it was given one.
    pub fn name(&self) -> Option<&str> {
        self.root().get("name").and_then(KdlValue::as_string)
    }

    /// The schema version the file declares.
    pub fn version(&self) -> u64 {
        self.root()
            .get("version")
            .and_then(KdlValue::as_integer)
            .and_then(|v| u64::try_from(v).ok())
            .expect("checked at parse")
    }

    /// What this form is for. [`FormKind::Screen`] unless the file says otherwise.
    pub fn kind(&self) -> FormKind {
        self.root()
            .get("kind")
            .and_then(KdlValue::as_string)
            .and_then(FormKind::from_name)
            .unwrap_or(FormKind::Screen)
    }

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

    /// The theme the file names, or the dark one.
    pub fn theme(&self) -> Theme {
        match self.theme_name() {
            "light" => theme::LIGHT,
            "high-contrast" => theme::HIGH_CONTRAST,
            _ => theme::DARK,
        }
    }

    /// The theme's name, as the file spells it.
    pub fn theme_name(&self) -> &str {
        self.root()
            .get("theme")
            .and_then(KdlValue::as_string)
            .unwrap_or("dark")
    }

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
    pub fn text(&self) -> String {
        self.doc.to_string()
    }

    // ------------------------------------------------------------- editing

    /// The node at a child path, if there is one.
    fn at_mut(&mut self, path: &[usize]) -> Option<&mut KdlNode> {
        let (&first, rest) = path.split_first()?;
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
            Some(node) => {
                node.insert(name, KdlValue::Integer(i128::from(value)));
                true
            }
            None => false,
        }
    }

    /// Removes a property from the node at `path`.
    ///
    /// What a designer does when a property goes back to its default: the schema
    /// says a default is not written, so resetting one is deleting it rather than
    /// spelling it out.
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

/// The offset of the brace that goes one level too deep, if there is one.
///
/// A scanner rather than a parse, because it has to run *before* the parser: it
/// skips comments and every shape of KDL string so that a `{` inside one is not
/// counted, and counts nothing else. Braces cannot appear in a bare identifier,
/// so what is left is structure.
fn too_deep(source: &str) -> Option<usize> {
    let b = source.as_bytes();
    let mut i = 0;
    let mut depth = 0usize;

    while i < b.len() {
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
                    i += 1;
                    while i < b.len() {
                        if b[i] == b'"' {
                            let mut end = i + 1;
                            let mut seen = 0usize;
                            while seen < hashes && b.get(end) == Some(&b'#') {
                                end += 1;
                                seen += 1;
                            }
                            if seen == hashes {
                                i = end;
                                break;
                            }
                        }
                        i += 1;
                    }
                }
            }
            b'"' => {
                if b[i..].starts_with(b"\"\"\"") {
                    i += 3;
                    while i < b.len() && !b[i..].starts_with(b"\"\"\"") {
                        i += 1;
                    }
                    i = i.saturating_add(3).min(b.len());
                } else {
                    i += 1;
                    while i < b.len() && b[i] != b'"' {
                        // An escaped character, including an escaped quote.
                        i += if b[i] == b'\\' { 2 } else { 1 };
                    }
                    i += 1;
                }
            }
            b'{' => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Some(i);
                }
                i += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nesting_within_the_limit_is_allowed() {
        let source = "a ".to_string() + &"{ b ".repeat(MAX_DEPTH) + &"}".repeat(MAX_DEPTH);
        assert_eq!(too_deep(&source), None);
    }

    #[test]
    fn one_level_past_the_limit_is_caught_before_the_parser_sees_it() {
        let deep = MAX_DEPTH + 1;
        let source = "a ".to_string() + &"{ b ".repeat(deep) + &"}".repeat(deep);
        assert!(too_deep(&source).is_some());
    }

    #[test]
    fn a_brace_inside_a_string_is_not_structure() {
        for source in [
            r#"a "{{{{{{{{{{{{{{{{" b"#,
            r##"a #"{{{{{{{{{{{{{{{{"# b"##,
            r#"a ""{{{{{{{{{{{{{{{{" b"#,
            "a // {{{{{{{{{{{{{{{{{{{{{{{{{{{{\n b",
            "a /* {{{{{{{{{{{{{{{{{{{{{{{{{{ */ b",
        ] {
            assert_eq!(too_deep(source), None, "in {source}");
        }
    }

    #[test]
    fn an_unterminated_string_does_not_loop_forever() {
        assert_eq!(too_deep("a \"unterminated"), None);
        assert_eq!(too_deep("a #\"unterminated"), None);
        assert_eq!(too_deep("a /* unterminated"), None);
    }
}
