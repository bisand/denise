//! Turning a parsed form into a live widget tree.

use std::collections::HashMap;

use denise::{Rect, Size};
use denise_ui::widgets::describe::{
    ALIGNMENTS, FITS, ORIENTATIONS, PRESENCES, Payload, Property, PropertyKind, RADII, ROLES,
    Value, WidgetInfo, role_from_name,
};
use denise_ui::widgets::{
    Alert, Avatar, Badge, Button, Carousel, Checkbox, Collapse, Column, Divider, Fit, Image, Label,
    List, ListItem, Panel, Progress, RadialProgress, RadioGroup, Rating, Select, Slider, Spinner,
    Table, Tabs, TextInput, Timeline, TimelineItem, Toggle, Video,
};
use denise_ui::{Anchors, Dock, NodeId, Ui};
use kdl::{KdlNode, KdlValue};

use crate::error::{At, Error, Reason};
use crate::form::{Form, FormKind, MAX_DEPTH};

/// Pixels for a picture a form named, as [`Wiring::asset`] hands them back.
#[derive(Clone, Debug)]
pub struct Picture {
    /// Premultiplied `0xAARRGGBB`, which is `denise-ui`'s contract exactly.
    pub pixels: Vec<u32>,
    /// The picture's own size.
    pub size: Size,
}

/// A message a form named, in the shape the widget holding it needs.
///
/// Widgets do not all take a message the same way, and none of them takes a
/// closure: a `Button` holds an `M`, a `Checkbox` a `fn(bool) -> M`, a `List` a
/// `fn(usize) -> M`, a `Slider` a `fn(f32) -> M`. Those are **function
/// pointers**, so nothing this crate could build from a name would fit — but an
/// enum's tuple variant already is one:
///
/// ```
/// # use denise_forms::Handler;
/// #[derive(Clone, Copy)]
/// enum Message {
///     Save,
///     Notify(bool),
/// }
///
/// let save = Handler::Plain(Message::Save);
/// // `Message::Notify` *is* a `fn(bool) -> Message`.
/// let notify = Handler::Bool(Message::Notify);
/// # let _ = (save, notify);
/// ```
#[derive(Clone, Copy, Debug)]
pub enum Handler<M> {
    /// The message itself, for a widget that holds one: a button, a select, a
    /// text field's submit.
    Plain(M),
    /// `fn(bool) -> M` — a checkbox, a toggle, a collapse.
    Bool(fn(bool) -> M),
    /// `fn(usize) -> M` — anything that selects one of several.
    Index(fn(usize) -> M),
    /// `fn(f32) -> M` — a slider, a rating.
    Number(fn(f32) -> M),
}

impl<M> Handler<M> {
    fn wanted(payload: Payload) -> &'static str {
        match payload {
            Payload::None => "the message itself",
            Payload::Bool => "a `fn(bool) -> M`",
            Payload::Index => "a `fn(usize) -> M`",
            Payload::Number => "a `fn(f32) -> M`",
        }
    }
}

/// What an application supplies a form that this crate cannot: its own message
/// type, and its own pictures.
///
/// A plain closure implements this, which is all most forms need. Implement it on
/// a type when the form also names pictures.
pub trait Wiring<M> {
    /// Turns a message name from the file into a message of the application's
    /// own type. `payload` says which shape the widget needs.
    fn message(&mut self, name: &str, payload: Payload) -> Option<Handler<M>>;

    /// Loads a picture, by a path **relative to the form file**.
    ///
    /// The default has none, so a form naming a picture in an application that
    /// supplied no loader fails with the path in the message rather than drawing
    /// a hole. This crate decodes nothing and does not depend on `denise-image`:
    /// that keeps a board with its pictures compiled in from linking a decoder it
    /// will never call.
    fn asset(&mut self, path: &str) -> Option<Picture> {
        let _ = path;
        None
    }
}

impl<M, F> Wiring<M> for F
where
    F: FnMut(&str, Payload) -> Option<Handler<M>>,
{
    fn message(&mut self, name: &str, payload: Payload) -> Option<Handler<M>> {
        self(name, payload)
    }
}

/// One node the form put in the tree, and where in the file it came from.
///
/// A designer needs both halves: the [`NodeId`] to hit-test and draw a selection
/// around, and the [`path`](Placed::path) to edit when the selection moves. The
/// path is a list of child indices from the `form` node down, which is stable
/// across a rebuild in a way a byte offset is not — every edit shifts the offsets
/// after it, and the whole point is to edit and carry on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    /// The node in the tree.
    pub id: NodeId,
    /// Its parent in the tree, or `None` for a node directly under the form.
    pub parent: Option<NodeId>,
    /// What kind of widget it is.
    pub kind: &'static str,
    /// The name the file gave it, if it gave one.
    pub name: Option<String>,
    /// Child indices from the `form` node's children down to this node.
    pub path: Vec<usize>,
}

/// What a form built, so an application can find what it made.
#[derive(Clone, Debug, Default)]
pub struct Built {
    names: HashMap<String, NodeId>,
    placed: Vec<Placed>,
}

impl Built {
    /// See [`Form::build`] for one of these being made and read.
    /// The node a form gave this name, if it gave one that name.
    pub fn node(&self, name: &str) -> Option<NodeId> {
        self.names.get(name).copied()
    }

    /// See [`Form::build`] for one of these being made and read.
    /// Every name the form gave a node, in no particular order.
    pub fn names(&self) -> impl Iterator<Item = (&str, NodeId)> {
        self.names.iter().map(|(name, &id)| (name.as_str(), id))
    }

    /// Every node the form built, in file order.
    ///
    /// See [`Form::build`] for one of these being made and read.
    /// Includes the ones with no name: a designer selects what a person clicked
    /// on, and most of what a person clicks on was never named.
    pub fn placed(&self) -> &[Placed] {
        &self.placed
    }

    /// See [`Form::build`] for one of these being made and read.
    /// The node at a path, if the form put one there.
    pub fn at(&self, path: &[usize]) -> Option<&Placed> {
        self.placed.iter().find(|p| p.path == path)
    }

    /// See [`Form::build`] for one of these being made and read.
    /// How many nodes were named.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// See [`Form::build`] for one of these being made and read.
    /// Whether the form named nothing.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

const ANCHOR_EDGES: &[&str] = &["left", "top", "right", "bottom"];
const DOCK_SIDES: &[&str] = &["top", "bottom", "left", "right", "fill"];

/// Anywhere on a form's surface, and then some.
///
/// A rectangle is advice to an editor, not a rule: a node may sit outside its
/// parent and the tree will clip it, which is occasionally what somebody means.
const ANYWHERE: PropertyKind = PropertyKind::Int {
    min: -8192,
    max: 8192,
};

/// The properties the `form` node itself carries, whatever kind it is.
///
/// Not `version`, which is the file format's rather than the form's and is not
/// somebody's to edit; and not the title, which is the node's *argument* rather
/// than a property. Everything else about a form is here, which is what lets an
/// inspector show a form the same way it shows a widget — from a descriptor,
/// with no list of its own.
pub const FORM_PROPERTIES: &[Property] = &[
    Property::new(
        "name",
        PropertyKind::Text,
        "What the application calls this form. Names what the typed layer generates.",
    ),
    Property::new(
        "kind",
        PropertyKind::Enum(FormKind::NAMES),
        "What this form is for: a screen, a window, a dialog, a drawer, a shelf, or a fragment.",
    ),
    Property::new(
        "width",
        PropertyKind::Int { min: 1, max: 8192 },
        "The width the form was designed at, in logical pixels.",
    ),
    Property::new(
        "height",
        PropertyKind::Int { min: 1, max: 8192 },
        "The height the form was designed at, in logical pixels.",
    ),
    Property::new(
        "theme",
        PropertyKind::Enum(crate::form::THEMES),
        "Which built-in theme the form is drawn with.",
    ),
    Property::new(
        "background",
        PropertyKind::Enum(denise_ui::widgets::ROLES),
        "The surface the form is drawn on.",
    ),
];

/// What only a window has.
const WINDOW_PROPERTIES: &[Property] = &[
    Property::new(
        "resizable",
        PropertyKind::Bool,
        "Whether the window may be resized. Windows only.",
    ),
    Property::new(
        "min-width",
        PropertyKind::Int { min: 0, max: 8192 },
        "The narrowest the window may be made. Windows only.",
    ),
    Property::new(
        "min-height",
        PropertyKind::Int { min: 0, max: 8192 },
        "The shortest the window may be made. Windows only.",
    ),
];

/// What only a dialog has.
const DIALOG_PROPERTIES: &[Property] = &[Property::new(
    "dim",
    PropertyKind::Int { min: 0, max: 255 },
    "How dark the backdrop behind the dialog is, 0 to 255. Dialogs only.",
)];

/// What comes in from an edge: a drawer and a shelf, which differ in modality
/// and not in shape.
const EDGE_PROPERTIES: &[Property] = &[
    Property::new(
        "side",
        PropertyKind::Enum(denise_ui::widgets::SIDES),
        "Which edge it comes in from.",
    ),
    Property::new(
        "extent",
        PropertyKind::Int { min: 1, max: 8192 },
        "How far it comes in. Required; across the other axis it covers the surface.",
    ),
];

/// The properties a form of this kind carries **and no other kind does**.
///
/// A `resizable` on a screen is not a property with no effect; it is a mistake,
/// and saying so is the whole reason this is a function of the kind rather than
/// one long list.
/// ```
/// # use denise_forms::{FORM_PROPERTIES, FormKind, form_property, kind_properties};
/// // Everything every form has.
/// assert!(FORM_PROPERTIES.iter().any(|it| it.name == "width"));
///
/// // And what only this kind has.
/// assert!(kind_properties(FormKind::Window).iter().any(|it| it.name == "resizable"));
/// assert!(kind_properties(FormKind::Screen).is_empty());
///
/// // `form_property` is the two together, which is what "may a form of this
/// // kind say this?" means.
/// assert!(form_property(FormKind::Window, "resizable").is_some());
/// assert!(form_property(FormKind::Screen, "resizable").is_none());
/// assert!(form_property(FormKind::Screen, "width").is_some());
/// ```
pub const fn kind_properties(kind: FormKind) -> &'static [Property] {
    match kind {
        FormKind::Window => WINDOW_PROPERTIES,
        FormKind::Dialog => DIALOG_PROPERTIES,
        FormKind::Drawer | FormKind::Shelf => EDGE_PROPERTIES,
        FormKind::Screen | FormKind::Fragment => &[],
    }
}

/// Whether the `form` node may carry this property, given its kind.
/// See [`kind_properties`].
pub fn form_property(kind: FormKind, name: &str) -> Option<&'static Property> {
    FORM_PROPERTIES
        .iter()
        .chain(kind_properties(kind))
        .find(|property| property.name == name)
}

/// The properties the *tree* owns rather than the widget.
///
/// Geometry, visibility, ordering, placement. A widget's descriptor never
/// mentions them, so they are checked against this list before a widget is asked
/// whether it has heard of them.
///
/// Described the same way a widget describes its own, and for the same reason:
/// the designer's inspector draws an editor per [`Property`] and has no table of
/// its own, so `x` and `dock` get one from here exactly as `role` gets one from
/// the widget.
pub const NODE_PROPERTIES: &[Property] = &[
    Property::new(
        "name",
        PropertyKind::Text,
        "What the application calls this node. Unique within the form.",
    ),
    Property::new("x", ANYWHERE, "Left edge, relative to the parent."),
    Property::new("y", ANYWHERE, "Top edge, relative to the parent."),
    Property::new(
        "w",
        PropertyKind::Int { min: 0, max: 8192 },
        "Width in pixels.",
    ),
    Property::new(
        "h",
        PropertyKind::Int { min: 0, max: 8192 },
        "Height in pixels.",
    ),
    Property::new(
        "visible",
        PropertyKind::Bool,
        "Drawn and able to be touched, or neither.",
    ),
    Property::new(
        "enabled",
        PropertyKind::Bool,
        "Takes input, or is greyed out and does not.",
    ),
    Property::new(
        "z",
        PropertyKind::Int {
            min: -1000,
            max: 1000,
        },
        "Paint order among siblings; higher is nearer the front.",
    ),
    Property::new(
        "tooltip",
        PropertyKind::Text,
        "What resting the pointer on this node says.",
    ),
    Property::new(
        "scroll",
        PropertyKind::Bool,
        "Whether children reaching past this node can be scrolled to.",
    ),
    Property::new(
        "stack",
        PropertyKind::Int { min: 0, max: 1000 },
        "Stacks the children down the node with this many pixels between them.",
    ),
    Property::new(
        "focus",
        PropertyKind::Bool,
        "Whether this node holds the caret when the form opens. One per form.",
    ),
    Property::new(
        "anchor",
        PropertyKind::Text,
        "Edges held as the parent resizes: any of left, top, right, bottom.",
    ),
    Property::new(
        "dock",
        PropertyKind::Enum(DOCK_SIDES),
        "An edge of the parent this node takes for itself, before the rest are placed.",
    ),
];

/// The tree-owned property of this name, if there is one.
/// ```
/// # use denise_forms::node_property;
/// // The tree owns geometry and visibility; no widget declares them.
/// assert!(node_property("x").is_some());
/// assert!(node_property("dock").is_some());
/// // A widget's own property is not one of these.
/// assert!(node_property("role").is_none());
/// ```
pub fn node_property(name: &str) -> Option<&'static Property> {
    NODE_PROPERTIES.iter().find(|p| p.name == name)
}

/// Child nodes that are a parent's *content* rather than nodes of their own.
const COLLECTIONS: &[&str] = &["option", "item", "column", "row", "event", "picture", "tab"];

/// Whether a widget of this kind can hold nodes of their own.
///
/// Two do. Everything else either has no children or has *content* — a `select`
/// holds `option`s, a `table` holds `column`s — which is not the same thing: a
/// designer dropping a button on a `select` has missed, and dropping one on a
/// `panel` means it.
/// ```
/// # use denise_forms::owns_children;
/// assert!(owns_children("panel"));
/// assert!(owns_children("collapse"));
/// // Content is not children: a `select` holds options, and dropping a button
/// // on one has missed.
/// assert!(!owns_children("select"));
/// assert!(!owns_children("label"));
/// ```
pub fn owns_children(kind: &str) -> bool {
    matches!(kind, "panel" | "collapse")
}

/// The kinds that carry their text as the node's argument.
///
/// `label "Heading"` rather than `label text="Heading"`. Both build the same
/// thing; the first is how every form in this repository is written, and is what
/// [`seed`] produces.
const ARGUMENT: &[&str] = &[
    "label", "badge", "divider", "alert", "button", "checkbox", "toggle", "collapse",
];

/// How big a new widget of this kind should start out.
///
/// **Authoring defaults, not intrinsic sizes.** This toolkit has no layout engine
/// and nothing here has a size of its own: a button is whatever rectangle the
/// form gives it. These are the rectangles that make a dropped widget look like
/// what it is, so that somebody can see what they placed before they resize it —
/// which is a question about writing forms, and so this crate's, rather than a
/// question about widgets.
/// ```
/// # use denise_forms::default_size;
/// // A button is wider than it is tall; an avatar is square.
/// let button = default_size("button");
/// assert!(button.width > button.height);
/// let avatar = default_size("avatar");
/// assert_eq!(avatar.width, avatar.height);
/// // A kind nobody has heard of still gets something you can see and click.
/// assert!(default_size("banana").width > 0);
/// ```
pub fn default_size(kind: &str) -> Size {
    let (width, height) = match kind {
        "alert" => (320, 36),
        "avatar" => (40, 40),
        "badge" => (60, 20),
        "button" => (100, 32),
        "carousel" => (224, 120),
        "checkbox" | "toggle" => (200, 24),
        "collapse" => (224, 40),
        "divider" => (160, 16),
        "image" => (120, 90),
        "list" => (200, 160),
        "panel" => (200, 120),
        "progress" => (200, 8),
        "radial-progress" => (48, 48),
        "radio-group" => (220, 76),
        "rating" => (140, 24),
        "select" | "text-input" => (220, 34),
        "slider" => (200, 24),
        "spinner" => (24, 24),
        "table" => (320, 180),
        "tabs" => (320, 36),
        "timeline" => (220, 140),
        "video" => (160, 90),
        // `label`, and anything this list has not heard of.
        _ => (120, 20),
    };
    Size::new(width, height)
}

/// The smallest node of this kind that a form can actually hold, as file text.
///
/// What a designer writes when somebody drops a widget on the canvas. A rectangle
/// is the most of it — but "a rect and nothing else" is not true of every widget,
/// because five of them have a property the builder *requires*: an `alert` has no
/// colour to draw itself in without a `role`, a `slider` has no range without
/// `min` and `max`, and `select` and `collapse` have no inert constructor to fall
/// back on. A node missing one of those parses and then will not build, so a
/// designer that wrote one would place a widget and break the form.
///
/// This lives beside the code that raises those requirements, so the two cannot
/// drift; a test seeds every widget in [`all`](denise_ui::widgets::all), builds
/// the result, and fails if a new one needs something this does not give it.
///
/// ```
/// # use denise_forms::{seed, Form};
/// use denise::Rect;
///
/// assert_eq!(
///     seed("button", Rect::new(16, 24, 100, 32)),
///     r#"button "button" x=16 y=24 w=100 h=32"#,
/// );
/// ```
pub fn seed(kind: &str, rect: Rect) -> String {
    let mut node = String::from(kind);
    if ARGUMENT.contains(&kind) {
        // The kind, as a placeholder. A label dropped with nothing to say draws
        // nothing, and a widget you cannot see is a widget you cannot find
        // again the moment you click somewhere else.
        node.push_str(&format!(" {:?}", kind));
    }
    node.push_str(&format!(
        " x={} y={} w={} h={}",
        rect.x, rect.y, rect.width, rect.height
    ));
    node.push_str(match kind {
        "alert" => " role=info",
        "slider" => " min=0 max=100",
        // Neither has an inert constructor: both hold a plain message, and there
        // is no message a form file could invent. The name is a placeholder the
        // application will rename.
        "select" => " on-change=changed",
        "collapse" => " on-toggle=toggled",
        // A path that is not there yet. An engine that cannot load it says so;
        // a designer draws a hole and carries on.
        "image" => " src=\"picture.png\"",
        _ => "",
    });
    node
}

/// A whole form file with nothing in it yet, writing **only** what is not a
/// default.
///
/// What *File → New* produces. A form that spelled out every default would read
/// as a form somebody had made decisions about, and the next person would have
/// to check each one against the schema to find out that none of them meant
/// anything. The exception is `extent`, which a drawer and a shelf must say:
/// this picks a third of the axis it comes in along, which is a drawer somebody
/// will recognise rather than one they have to fix before they can see it.
/// ```
/// # use denise::Size;
/// # use denise_forms::{Form, FormKind, seed_form};
/// // A screen is every default but its size, so it says nothing else.
/// let screen = seed_form("Untitled", FormKind::Screen, Size::new(800, 480));
/// assert_eq!(screen, "form \"Untitled\" version=1 width=800 height=480\n");
///
/// // What comes in from an edge has to say how far, so this picks one.
/// let drawer = seed_form("Filters", FormKind::Drawer, Size::new(1024, 600));
/// let form = Form::parse(&drawer)?;
/// assert_eq!(form.kind(), FormKind::Drawer);
/// assert_eq!(form.extent(), 1024 / 3);
/// # Ok::<(), denise_forms::Error>(())
/// ```
pub fn seed_form(title: &str, kind: FormKind, size: Size) -> String {
    let mut out = format!("form {title:?} version={}", crate::form::VERSION);
    if kind != FormKind::Screen {
        out.push_str(&format!(" kind={}", FormKind::NAMES[kind as usize]));
    }
    out.push_str(&format!(" width={} height={}", size.width, size.height));
    if matches!(kind, FormKind::Drawer | FormKind::Shelf) {
        let along = match kind.default_side() {
            denise_ui::Side::Above | denise_ui::Side::Below => size.height,
            denise_ui::Side::Before | denise_ui::Side::After => size.width,
        };
        out.push_str(&format!(" extent={}", (along / 3).max(1)));
    }
    out.push('\n');
    out
}

impl Form {
    /// ```
    /// # use denise_forms::{Form, Handler, Payload};
    /// # use denise_ui::Ui;
    /// #[derive(Clone, Copy, PartialEq, Debug)]
    /// enum Message {
    ///     Greet,
    /// }
    ///
    /// let form = Form::parse(
    ///     r#"form "Hello" version=1 width=320 height=120 { button "Greet" name=go x=8 y=8 w=90 h=30 on-press=greet }"#,
    /// )?;
    ///
    /// let mut ui: Ui<Message> = Ui::new(form.size(), form.theme());
    /// let root = ui.root();
    ///
    /// // The one thing a file cannot hold: this application's own message type.
    /// let built = form.build(&mut ui, root, &mut |name: &str, payload: Payload| {
    ///     match (name, payload) {
    ///         ("greet", Payload::None) => Some(Handler::Plain(Message::Greet)),
    ///         _ => None,
    ///     }
    /// })?;
    ///
    /// // What the file named, by the name it used.
    /// let button = built.node("go").expect("the form names it `go`");
    /// assert_eq!(built.len(), 1);
    /// assert!(!built.is_empty());
    ///
    /// // And everything it put on screen, named or not, in file order.
    /// assert_eq!(built.placed().len(), 1);
    /// assert_eq!(built.at(&[0]).map(|node| node.kind), Some("button"));
    /// assert_eq!(built.at(&[0]).map(|node| node.id), Some(button));
    /// assert_eq!(
    ///     built.names().map(|(name, _)| name).collect::<Vec<_>>(),
    ///     vec!["go"],
    /// );
    /// # Ok::<(), denise_forms::Error>(())
    /// ```
    ///
    /// Builds this form into `ui` under `parent`.
    ///
    /// Nodes are added in file order, so paint order is file order. See the
    /// [crate documentation](crate) for what `wiring` supplies and why.
    ///
    /// # Errors
    ///
    /// Every failure carries a line and a column. See [`Reason`](crate::Reason)
    /// for the whole list.
    pub fn build<M: Clone + 'static>(
        &self,
        ui: &mut Ui<M>,
        parent: NodeId,
        wiring: &mut impl Wiring<M>,
    ) -> Result<Built, Error> {
        let mut builder = Builder {
            form: self,
            ui,
            wiring,
            built: Built::default(),
            focused: None,
        };
        let children: Vec<&KdlNode> = self
            .root()
            .children()
            .map(|d| d.nodes().iter().collect())
            .unwrap_or_default();
        for (index, node) in children.into_iter().enumerate() {
            builder.node(node, parent, 0, &[index])?;
        }
        // The caret goes last, once every node exists: a form may name a field
        // that appears after the one before it in the file.
        let focused = builder.focused;
        let built = builder.built;
        if let Some(id) = focused {
            ui.focus(Some(id));
        }
        Ok(built)
    }
}

struct Builder<'a, M: 'static, W> {
    form: &'a Form,
    ui: &'a mut Ui<M>,
    wiring: &'a mut W,
    built: Built,
    focused: Option<NodeId>,
}

impl<M: Clone + 'static, W: Wiring<M>> Builder<'_, M, W> {
    fn err(&self, node: &KdlNode, reason: Reason) -> Error {
        Error::new(self.form.at_node(node), reason)
    }

    /// Builds one node and everything under it.
    fn node(
        &mut self,
        node: &KdlNode,
        parent: NodeId,
        depth: usize,
        path: &[usize],
    ) -> Result<(), Error> {
        if depth >= MAX_DEPTH {
            return Err(self.err(node, Reason::TooDeep { limit: MAX_DEPTH }));
        }
        let kind = node.name().value();
        if COLLECTIONS.contains(&kind) {
            // Reaching here means a collection node is somewhere its parent does
            // not read it — `option` outside a `select`, say — which is a typo
            // that would otherwise vanish silently.
            return Err(self.err(
                node,
                Reason::UnexpectedChild {
                    parent: String::from("form"),
                    found: kind.to_string(),
                },
            ));
        }
        let info = *denise_ui::widgets::all()
            .iter()
            .find(|w| w.kind == kind)
            .ok_or_else(|| {
                self.err(
                    node,
                    Reason::UnknownWidget {
                        found: kind.to_string(),
                    },
                )
            })?;

        self.check_properties(node, &info)?;
        let rect = self.rect(node)?;
        let id = self.construct(node, &info, parent, rect)?;
        self.apply_properties(node, &info, id)?;
        self.apply_node_properties(node, id)?;
        self.built.placed.push(Placed {
            id,
            parent: (depth > 0).then_some(parent),
            kind: info.kind,
            name: self.string(node, "name"),
            path: path.to_vec(),
        });

        // Children that are not the parent's own content are nodes in their own
        // right. A widget that cannot lay children out says so.
        if let Some(children) = node.children() {
            let owns_children = owns_children(kind);
            for (index, child) in children.nodes().iter().enumerate() {
                let name = child.name().value();
                if COLLECTIONS.contains(&name) {
                    continue;
                }
                if !owns_children {
                    return Err(self.err(
                        child,
                        Reason::UnexpectedChild {
                            parent: kind.to_string(),
                            found: name.to_string(),
                        },
                    ));
                }
                let mut below = path.to_vec();
                below.push(index);
                self.node(child, id, depth + 1, &below)?;
            }
        }
        Ok(())
    }

    /// Every property in the file is one the tree owns or one the widget declares.
    fn check_properties(&self, node: &KdlNode, info: &WidgetInfo) -> Result<(), Error> {
        for entry in node.entries() {
            let Some(name) = entry.name() else {
                continue;
            };
            let name = name.value();
            if node_property(name).is_some() || info.property(name).is_some() {
                continue;
            }
            return Err(Error::new(
                self.form.at(entry.span().offset()),
                Reason::UnknownProperty {
                    kind: info.kind,
                    found: name.to_string(),
                    accepted: info.properties,
                },
            ));
        }
        Ok(())
    }

    fn rect(&self, node: &KdlNode) -> Result<Rect, Error> {
        let mut axes = [0i32; 4];
        for (slot, name) in axes.iter_mut().zip(["x", "y", "w", "h"]) {
            let value = node
                .get(name)
                .and_then(KdlValue::as_integer)
                .ok_or_else(|| {
                    self.err(
                        node,
                        Reason::Missing {
                            kind: node.name().value().to_string(),
                            name: match name {
                                "x" => "x",
                                "y" => "y",
                                "w" => "w",
                                _ => "h",
                            },
                        },
                    )
                })?;
            *slot = i32::try_from(value).unwrap_or(i32::MAX);
        }
        Ok(Rect::new(axes[0], axes[1], axes[2], axes[3]))
    }

    /// The node's single positional argument, as a string.
    fn arg(&self, node: &KdlNode) -> Option<String> {
        node.entries()
            .iter()
            .find(|e| e.name().is_none())
            .and_then(|e| e.value().as_string())
            .map(str::to_string)
    }

    fn string(&self, node: &KdlNode, name: &str) -> Option<String> {
        node.get(name)
            .and_then(KdlValue::as_string)
            .map(str::to_string)
    }

    fn number(&self, node: &KdlNode, name: &str) -> Option<f32> {
        node.get(name).and_then(|v| {
            v.as_float()
                .map(|f| f as f32)
                .or_else(|| v.as_integer().map(|i| i as f32))
        })
    }

    /// A message the file named, in the shape this widget needs.
    fn handler(
        &mut self,
        node: &KdlNode,
        property: &str,
        payload: Payload,
    ) -> Result<Option<Handler<M>>, Error> {
        let Some(name) = self.string(node, property) else {
            return Ok(None);
        };
        match self.wiring.message(&name, payload) {
            Some(handler) => Ok(Some(handler)),
            None => Err(self.err(node, Reason::UnknownMessage { found: name })),
        }
    }

    fn plain(&self, node: &KdlNode, name: &str, handler: Handler<M>) -> Result<M, Error> {
        match handler {
            Handler::Plain(message) => Ok(message),
            _ => Err(self.wrong(node, name, Payload::None)),
        }
    }

    fn on_bool(
        &self,
        node: &KdlNode,
        name: &str,
        handler: Handler<M>,
    ) -> Result<fn(bool) -> M, Error> {
        match handler {
            Handler::Bool(f) => Ok(f),
            _ => Err(self.wrong(node, name, Payload::Bool)),
        }
    }

    fn on_index(
        &self,
        node: &KdlNode,
        name: &str,
        handler: Handler<M>,
    ) -> Result<fn(usize) -> M, Error> {
        match handler {
            Handler::Index(f) => Ok(f),
            _ => Err(self.wrong(node, name, Payload::Index)),
        }
    }

    fn on_number(
        &self,
        node: &KdlNode,
        name: &str,
        handler: Handler<M>,
    ) -> Result<fn(f32) -> M, Error> {
        match handler {
            Handler::Number(f) => Ok(f),
            _ => Err(self.wrong(node, name, Payload::Number)),
        }
    }

    fn wrong(&self, node: &KdlNode, property: &str, payload: Payload) -> Error {
        self.err(
            node,
            Reason::WrongMessage {
                found: self.string(node, property).unwrap_or_default(),
                wanted: Handler::<M>::wanted(payload),
            },
        )
    }

    fn required(&self, node: &KdlNode, name: &'static str) -> Error {
        self.err(
            node,
            Reason::Missing {
                kind: node.name().value().to_string(),
                name,
            },
        )
    }

    /// The child nodes of one collection kind.
    fn collection<'n>(&self, node: &'n KdlNode, name: &str) -> Vec<&'n KdlNode> {
        node.children()
            .map(|d| {
                d.nodes()
                    .iter()
                    .filter(|n| n.name().value() == name)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn strings(&self, node: &KdlNode, name: &str) -> Vec<String> {
        self.collection(node, name)
            .into_iter()
            .map(|n| self.arg(n).unwrap_or_default())
            .collect()
    }

    fn picture(&mut self, node: &KdlNode, path: &str) -> Result<Picture, Error> {
        self.wiring.asset(path).ok_or_else(|| {
            Error::new(
                self.form.at_node(node),
                Reason::Asset {
                    path: path.to_string(),
                },
            )
        })
    }
}

// The construction match is long because there are twenty-five widgets and no
// two constructors are alike. It is deliberately not clever: a table of
// constructors would need one type for all of them, and they differ in exactly
// the way that would make that a lie.
impl<M: Clone + 'static, W: Wiring<M>> Builder<'_, M, W> {
    fn construct(
        &mut self,
        node: &KdlNode,
        info: &WidgetInfo,
        parent: NodeId,
        rect: Rect,
    ) -> Result<NodeId, Error> {
        let text = self.arg(node).unwrap_or_default();
        let id = match info.kind {
            "label" => self.ui.add(parent, Label::new(text), rect),
            "panel" => self.ui.add(parent, Panel::default(), rect),
            "badge" => self.ui.add(parent, Badge::new(text), rect),
            "divider" => {
                let divider = if self.arg(node).is_some() {
                    Divider::labelled(text)
                } else {
                    Divider::new()
                };
                self.ui.add(parent, divider, rect)
            }
            "alert" => {
                let role = self
                    .string(node, "role")
                    .ok_or_else(|| self.required(node, "role"))?;
                let role = role_from_name(&role).ok_or_else(|| {
                    self.err(
                        node,
                        Reason::NotAName {
                            name: String::from("colour role"),
                            found: role.clone(),
                            accepted: ROLES,
                        },
                    )
                })?;
                self.ui.add(parent, Alert::new(role, text), rect)
            }
            "spinner" => self.ui.add(parent, Spinner::new(), rect),
            "video" => self.ui.add(parent, Video::new(), rect),
            "progress" => {
                let value = self.number(node, "value").unwrap_or(0.0);
                self.ui.add(parent, Progress::new(value), rect)
            }
            "radial-progress" => {
                let value = self.number(node, "value").unwrap_or(0.0);
                self.ui.add(parent, RadialProgress::new(value), rect)
            }
            "button" => {
                let button = match self.handler(node, "on-press", Payload::None)? {
                    Some(h) => Button::new(text, self.plain(node, "on-press", h)?),
                    None => Button::inert(text),
                };
                self.ui.add(parent, button, rect)
            }
            "text-input" => {
                let mut field = TextInput::<M>::new();
                if let Some(h) = self.handler(node, "on-submit", Payload::None)? {
                    field = field.with_submit(self.plain(node, "on-submit", h)?);
                }
                self.ui.add(parent, field, rect)
            }
            "checkbox" => {
                let widget = match self.handler(node, "on-change", Payload::Bool)? {
                    Some(h) => Checkbox::new(text, self.on_bool(node, "on-change", h)?),
                    None => Checkbox::inert(text),
                };
                self.ui.add(parent, widget, rect)
            }
            "toggle" => {
                let widget = match self.handler(node, "on-change", Payload::Bool)? {
                    Some(h) => Toggle::new(text, self.on_bool(node, "on-change", h)?),
                    None => Toggle::inert(text),
                };
                self.ui.add(parent, widget, rect)
            }
            "slider" => {
                let min = self
                    .number(node, "min")
                    .ok_or_else(|| self.required(node, "min"))?;
                let max = self
                    .number(node, "max")
                    .ok_or_else(|| self.required(node, "max"))?;
                let value = self.number(node, "value").unwrap_or(min);
                let widget = match self.handler(node, "on-change", Payload::Number)? {
                    Some(h) => Slider::new(min, max, value, self.on_number(node, "on-change", h)?),
                    None => Slider::inert(min, max, value),
                };
                self.ui.add(parent, widget, rect)
            }
            "rating" => {
                let value = self.number(node, "value").unwrap_or(0.0);
                let widget = match self.handler(node, "on-change", Payload::Number)? {
                    Some(h) => Rating::new(value, self.on_number(node, "on-change", h)?),
                    None => Rating::display(value),
                };
                self.ui.add(parent, widget, rect)
            }
            "radio-group" => {
                let options = self.strings(node, "option");
                let widget = match self.handler(node, "on-change", Payload::Index)? {
                    Some(h) => RadioGroup::new(options, self.on_index(node, "on-change", h)?),
                    None => RadioGroup::inert(options),
                };
                self.ui.add(parent, widget, rect)
            }
            "tabs" => {
                let labels = self.strings(node, "tab");
                let widget = match self.handler(node, "on-change", Payload::Index)? {
                    Some(h) => Tabs::new(labels, self.on_index(node, "on-change", h)?),
                    None => Tabs::inert(labels),
                };
                self.ui.add(parent, widget, rect)
            }
            "select" => {
                let options = self.strings(node, "option");
                // `Select` has no inert constructor: it holds a plain message and
                // there is no message a form file could invent.
                let handler = self
                    .handler(node, "on-change", Payload::None)?
                    .ok_or_else(|| self.required(node, "on-change"))?;
                let widget = Select::new(options, self.plain(node, "on-change", handler)?);
                self.ui.add(parent, widget, rect)
            }
            "collapse" => {
                // Likewise: `Collapse` has no inert constructor.
                let handler = self
                    .handler(node, "on-toggle", Payload::Bool)?
                    .ok_or_else(|| self.required(node, "on-toggle"))?;
                let widget = Collapse::new(text, self.on_bool(node, "on-toggle", handler)?);
                self.ui.add(parent, widget, rect)
            }
            "list" => {
                let items: Vec<ListItem> = self
                    .collection(node, "item")
                    .into_iter()
                    .map(|n| {
                        let mut item = ListItem::new(self.arg(n).unwrap_or_default());
                        if let Some(leading) = self.string(n, "leading") {
                            item = item.with_leading(leading);
                        }
                        if let Some(trailing) = self.string(n, "trailing") {
                            item = item.with_trailing(trailing);
                        }
                        if n.get("enabled").and_then(KdlValue::as_bool) == Some(false) {
                            item = item.disabled();
                        }
                        item
                    })
                    .collect();
                let mut widget = match self.handler(node, "on-select", Payload::Index)? {
                    Some(h) => List::new(items, self.on_index(node, "on-select", h)?),
                    None => List::inert(items),
                };
                if let Some(h) = self.handler(node, "on-activate", Payload::Index)? {
                    widget = widget.on_activate(self.on_index(node, "on-activate", h)?);
                }
                self.ui.add(parent, widget, rect)
            }
            "table" => {
                let columns: Vec<Column> = self
                    .collection(node, "column")
                    .into_iter()
                    .map(|n| {
                        let title = self.arg(n).unwrap_or_default();
                        let mut column = match n.get("width").and_then(KdlValue::as_integer) {
                            Some(width) => Column::new(title, width as i32),
                            None => Column::flex(title),
                        };
                        match n.get("align").and_then(KdlValue::as_string) {
                            Some("end") => column = column.align_end(),
                            Some("center") => column = column.align_center(),
                            _ => {}
                        }
                        column
                    })
                    .collect();
                let rows: Vec<Vec<String>> = self
                    .collection(node, "row")
                    .into_iter()
                    .map(|n| {
                        n.entries()
                            .iter()
                            .filter(|e| e.name().is_none())
                            .map(|e| e.value().as_string().unwrap_or_default().to_string())
                            .collect()
                    })
                    .collect();
                let mut widget = match self.handler(node, "on-select", Payload::Index)? {
                    Some(h) => Table::new(columns, self.on_index(node, "on-select", h)?),
                    None => Table::inert(columns),
                };
                widget = widget.with_rows(rows);
                if let Some(h) = self.handler(node, "on-activate", Payload::Index)? {
                    widget = widget.on_activate(self.on_index(node, "on-activate", h)?);
                }
                self.ui.add(parent, widget, rect)
            }
            "timeline" => {
                let events: Vec<TimelineItem> = self
                    .collection(node, "event")
                    .into_iter()
                    .map(|n| {
                        let mut item = TimelineItem::new(self.arg(n).unwrap_or_default());
                        if let Some(time) = self.string(n, "time") {
                            item = item.with_time(time);
                        }
                        if let Some(role) =
                            self.string(n, "role").as_deref().and_then(role_from_name)
                        {
                            item = item.with_role(role);
                        }
                        if n.get("pending").and_then(KdlValue::as_bool) == Some(true) {
                            item = item.pending();
                        }
                        item
                    })
                    .collect();
                self.ui.add(parent, Timeline::new(events), rect)
            }
            "image" => {
                let path = self
                    .string(node, "src")
                    .ok_or_else(|| self.required(node, "src"))?;
                let picture = self.picture(node, &path)?;
                self.ui
                    .add(parent, Image::new(picture.pixels, picture.size), rect)
            }
            "avatar" => {
                let avatar = match self.string(node, "src") {
                    Some(path) => {
                        let picture = self.picture(node, &path)?;
                        Avatar::new(picture.pixels, picture.size)
                    }
                    None => {
                        Avatar::initials(self.string(node, "initials").unwrap_or_default().as_str())
                    }
                };
                self.ui.add(parent, avatar, rect)
            }
            "carousel" => {
                let mut widget = match self.handler(node, "on-change", Payload::Index)? {
                    Some(h) => Carousel::new(self.on_index(node, "on-change", h)?),
                    None => Carousel::inert(),
                };
                for picture_node in self.collection(node, "picture") {
                    let path = self
                        .string(picture_node, "src")
                        .ok_or_else(|| self.required(picture_node, "src"))?;
                    let picture = self.picture(picture_node, &path)?;
                    let fit = match self.string(picture_node, "fit").as_deref() {
                        Some("fill") => Fit::Fill,
                        Some("cover") => Fit::Cover,
                        Some("center") => Fit::Center,
                        _ => Fit::Contain,
                    };
                    widget = widget.with_picture_fit(picture.pixels, picture.size, fit);
                }
                self.ui.add(parent, widget, rect)
            }
            other => {
                // `all()` matched a kind this match does not, which means a
                // widget joined the catalogue and not the builder.
                return Err(self.err(
                    node,
                    Reason::UnknownWidget {
                        found: other.to_string(),
                    },
                ));
            }
        };
        id.ok_or_else(|| self.err(node, Reason::TreeRefused))
    }

    /// Applies every widget property the file gives, **in descriptor order**.
    ///
    /// Descriptor order rather than file order, and deliberately: a slider's
    /// `value` is clamped into its `min`/`max`, so a file that wrote them the
    /// other way round would otherwise land somewhere else than one that did not.
    /// The widget publishes the order it wants to be told things in, and this
    /// obeys it — so two files that say the same thing build the same tree.
    fn apply_properties(
        &mut self,
        node: &KdlNode,
        info: &WidgetInfo,
        id: NodeId,
    ) -> Result<(), Error> {
        for property in info.properties {
            if !property.is_settable() {
                // A message or an asset; both were given to the constructor.
                continue;
            }
            let Some(entry) = node
                .entries()
                .iter()
                .find(|e| e.name().map(kdl::KdlIdentifier::value) == Some(property.name))
            else {
                continue;
            };
            let at = self.form.at(entry.span().offset());
            let value = self.convert(at, info.kind, property, entry.value())?;
            if let Some(Err(error)) = self.ui.set_property(id, property.name, value) {
                return Err(Error::new(
                    at,
                    Reason::WrongType {
                        kind: info.kind,
                        name: property.name.to_string(),
                        wanted: match error.mismatch {
                            denise_ui::widgets::Mismatch::WrongType { expected } => expected.noun(),
                            _ => "something else",
                        },
                    },
                ));
            }
        }
        Ok(())
    }

    /// A value from the file, in the shape the property takes.
    fn convert(
        &self,
        at: At,
        kind: &'static str,
        property: &Property,
        value: &KdlValue,
    ) -> Result<Value, Error> {
        let wrong = |wanted: &'static str| {
            Error::new(
                at,
                Reason::WrongType {
                    kind,
                    name: property.name.to_string(),
                    wanted,
                },
            )
        };
        Ok(match property.kind {
            PropertyKind::Text | PropertyKind::Color => {
                Value::text(value.as_string().ok_or_else(|| wrong("a string"))?)
            }
            PropertyKind::Bool => {
                Value::Bool(value.as_bool().ok_or_else(|| wrong("true or false"))?)
            }
            PropertyKind::Int { .. } => {
                let number = value.as_integer().ok_or_else(|| wrong("a whole number"))?;
                Value::Int(i32::try_from(number).map_err(|_| wrong("a whole number"))?)
            }
            PropertyKind::Float { .. } => {
                let number = value
                    .as_float()
                    .map(|f| f as f32)
                    .or_else(|| value.as_integer().map(|i| i as f32))
                    .ok_or_else(|| wrong("a number"))?;
                Value::Float(number)
            }
            PropertyKind::Enum(names) => {
                let found = value
                    .as_string()
                    .ok_or_else(|| wrong("one of the listed names"))?;
                let name = names.iter().copied().find(|n| *n == found).ok_or_else(|| {
                    Error::new(
                        at,
                        Reason::NotAName {
                            name: property.name.to_string(),
                            found: found.to_string(),
                            accepted: names,
                        },
                    )
                })?;
                Value::Enum(name)
            }
            // Filtered out by `is_settable` before this is reached.
            PropertyKind::Message(_) | PropertyKind::Asset => return Err(wrong("nothing here")),
            _ => return Err(wrong("a value this crate does not know")),
        })
    }

    /// The properties the tree owns.
    fn apply_node_properties(&mut self, node: &KdlNode, id: NodeId) -> Result<(), Error> {
        if let Some(name) = self.string(node, "name") {
            if self.built.names.contains_key(&name) {
                return Err(self.err(node, Reason::DuplicateName { name }));
            }
            self.built.names.insert(name, id);
        }
        if let Some(text) = self.string(node, "tooltip") {
            self.ui.set_tooltip(id, text);
        }
        if let Some(z) = node.get("z").and_then(KdlValue::as_integer) {
            self.ui.set_z(id, z as i32);
        }
        if node.get("scroll").and_then(KdlValue::as_bool) == Some(true) {
            self.ui.set_scrollable(id, true);
        }
        if let Some(spacing) = node.get("stack").and_then(KdlValue::as_integer) {
            self.ui.set_stack(id, spacing as i32);
        }
        if let Some(anchor) = self.string(node, "anchor") {
            let mut anchors = Anchors::new(false, false, false, false);
            for edge in anchor.split_whitespace() {
                match edge {
                    "left" => anchors.left = true,
                    "top" => anchors.top = true,
                    "right" => anchors.right = true,
                    "bottom" => anchors.bottom = true,
                    other => {
                        return Err(self.err(
                            node,
                            Reason::NotAName {
                                name: String::from("anchor edge"),
                                found: other.to_string(),
                                accepted: ANCHOR_EDGES,
                            },
                        ));
                    }
                }
            }
            self.ui.set_anchors(id, anchors);
        }
        if let Some(dock) = self.string(node, "dock") {
            let side = match dock.as_str() {
                "top" => Dock::Top,
                "bottom" => Dock::Bottom,
                "left" => Dock::Left,
                "right" => Dock::Right,
                "fill" => Dock::Fill,
                other => {
                    return Err(self.err(
                        node,
                        Reason::NotAName {
                            name: String::from("dock side"),
                            found: other.to_string(),
                            accepted: DOCK_SIDES,
                        },
                    ));
                }
            };
            self.ui.set_dock(id, Some(side));
        }
        if node.get("enabled").and_then(KdlValue::as_bool) == Some(false) {
            self.ui.set_enabled(id, false);
        }
        if node.get("focus").and_then(KdlValue::as_bool) == Some(true) {
            if self.focused.is_some() {
                return Err(self.err(node, Reason::TwoFocuses));
            }
            self.focused = Some(id);
        }
        // Last: a hidden node's children still had to be built and placed, and
        // hiding it first would have them laid out against a node with no bounds.
        if node.get("visible").and_then(KdlValue::as_bool) == Some(false) {
            self.ui.set_visible(id, false);
        }
        Ok(())
    }
}

// Unused-import guard: these name tables are the ones the schema documents, and
// referencing them here keeps a rename in `denise-ui` from silently drifting.
const _: &[&[&str]] = &[ALIGNMENTS, FITS, ORIENTATIONS, PRESENCES, RADII];
