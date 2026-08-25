//! What a widget's properties *are*, described by the widget itself.
//!
//! A form file names a widget and a property — `button` and `role=primary` — and
//! something has to turn those strings into `Button::set_role(Role::Primary)`.
//! The obvious way is a table in whatever does the turning. There would then be
//! two of them, because the form designer's property inspector needs the same
//! knowledge in order to show an editor per property, and both would drift from
//! the widgets and from each other the first time a widget grew a setting.
//!
//! So the widget owns the list. [`Describe`] is implemented next to each widget,
//! in the same file, and everything else reads it:
//!
//! - `denise-forms` builds a tree from a `.dform` by calling [`Describe::set`]
//!   once per property in the file.
//! - The designer's inspector renders one editor per [`Property`], choosing which
//!   from the [`PropertyKind`].
//! - [`all`] lists every widget that ships, so a palette does not name them —
//!   with [`Describe::DOC`] saying what each one *is* and [`Describe::GROUP`]
//!   saying which shelf it belongs on, so the palette does not describe or file
//!   them either.
//!
//! # The two properties a widget cannot hold
//!
//! Most of a widget's settings are its own. Two are not, and
//! [`Property::is_settable`] is how they say so.
//!
//! A **message** is a value of the application's type. A `Button<M>` holds an `M`
//! and this crate has never seen `M`, so no `Value` can carry one. The engine
//! resolves a name from the file into the application's message and hands it to
//! the constructor.
//!
//! An **asset** is a path. `Image` holds decoded pixels, not a filename, and this
//! crate does not decode anything. The engine loads the path and constructs from
//! the pixels.
//!
//! Both are still *described*, because the inspector must offer them and the
//! engine must not report them as typos. [`Describe::set`] refuses them with
//! [`Mismatch::Supplied`].
//!
//! # Ranges are for editors, not for validation
//!
//! `PropertyKind::Float { min, max }` tells an inspector to draw a slider between
//! two numbers. It is not a gate: a widget that clamps — and most do — clamps a
//! value from [`Describe::set`] exactly as it clamps one from its own setter, so
//! there is one rule about what a `Progress` of `2.0` means rather than two.
//!
//! # Example
//!
//! ```
//! use denise_ui::widgets::{Button, Describe, Value};
//! use denise_ui::Void;
//!
//! let mut button = Button::<Void>::inert("Save");
//! button.set("text", Value::text("Apply")).unwrap();
//! assert_eq!(button.get("text"), Some(Value::text("Apply")));
//!
//! // The list is the widget's, not ours.
//! assert!(Button::<Void>::PROPERTIES.iter().any(|p| p.name == "role"));
//!
//! // A typo names the widget, the property and what would have been accepted.
//! let error = button.set("colour", Value::text("red")).unwrap_err();
//! assert!(error.to_string().contains("button"));
//! assert!(error.to_string().contains("colour"));
//! ```

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use denise::{Radius, Role};

use super::{Align, Fit, Orientation, avatar::Presence};

// ---------------------------------------------------------------- name tables

/// Every [`Role`], in the spelling a form file uses.
pub const ROLES: &[&str] = &[
    "base-100",
    "base-200",
    "base-300",
    "base-content",
    "primary",
    "primary-content",
    "secondary",
    "secondary-content",
    "accent",
    "accent-content",
    "neutral",
    "neutral-content",
    "info",
    "info-content",
    "success",
    "success-content",
    "warning",
    "warning-content",
    "error",
    "error-content",
];

/// Every [`Radius`] token.
pub const RADII: &[&str] = &["selector", "field", "box"];

/// Every [`Align`].
pub const ALIGNMENTS: &[&str] = &["start", "center", "end"];

/// Every [`Orientation`].
pub const ORIENTATIONS: &[&str] = &["horizontal", "vertical"];

/// Every [`Fit`].
pub const FITS: &[&str] = &["fill", "contain", "cover", "center"];

/// Every [`Presence`].
pub const PRESENCES: &[&str] = &["online", "offline", "busy"];

/// Every [`Side`](crate::Side).
///
/// No widget takes one: the side a drawer or a shelf comes in from is a
/// property of the *form*, which is why the name table lives here with the
/// others rather than on a widget that would never use it.
pub const SIDES: &[&str] = &["above", "below", "before", "after"];

/// The [`Role`] a name stands for, and back again.
///
/// The table and the mapping are next to each other on purpose: a role added to
/// one and not the other is a compile error, not a name that silently fails to
/// parse.
pub const fn role_from_name(name: &str) -> Option<Role> {
    // `match` on a string is not `const`, so this walks the table.
    let bytes = name.as_bytes();
    let mut i = 0;
    while i < ROLES.len() {
        if const_eq(ROLES[i].as_bytes(), bytes) {
            return Some(ROLE_VALUES[i]);
        }
        i += 1;
    }
    None
}

const ROLE_VALUES: [Role; 20] = [
    Role::Base100,
    Role::Base200,
    Role::Base300,
    Role::BaseContent,
    Role::Primary,
    Role::PrimaryContent,
    Role::Secondary,
    Role::SecondaryContent,
    Role::Accent,
    Role::AccentContent,
    Role::Neutral,
    Role::NeutralContent,
    Role::Info,
    Role::InfoContent,
    Role::Success,
    Role::SuccessContent,
    Role::Warning,
    Role::WarningContent,
    Role::Error,
    Role::ErrorContent,
];

const fn const_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// The name for a [`Role`].
pub const fn role_name(role: Role) -> &'static str {
    ROLES[role as usize]
}

/// The name for a [`Radius`].
pub const fn radius_name(radius: Radius) -> &'static str {
    match radius {
        Radius::Selector => "selector",
        Radius::Field => "field",
        Radius::Box => "box",
    }
}

/// The [`Radius`] a name stands for.
pub fn radius_from_name(name: &str) -> Option<Radius> {
    Some(match name {
        "selector" => Radius::Selector,
        "field" => Radius::Field,
        "box" => Radius::Box,
        _ => return None,
    })
}

/// The name for an [`Align`].
pub const fn align_name(align: Align) -> &'static str {
    match align {
        Align::Start => "start",
        Align::Center => "center",
        Align::End => "end",
    }
}

/// The [`Align`] a name stands for.
pub fn align_from_name(name: &str) -> Option<Align> {
    Some(match name {
        "start" => Align::Start,
        "center" => Align::Center,
        "end" => Align::End,
        _ => return None,
    })
}

/// The name for an [`Orientation`].
pub const fn orientation_name(orientation: Orientation) -> &'static str {
    match orientation {
        Orientation::Horizontal => "horizontal",
        Orientation::Vertical => "vertical",
    }
}

/// The [`Orientation`] a name stands for.
pub fn orientation_from_name(name: &str) -> Option<Orientation> {
    Some(match name {
        "horizontal" => Orientation::Horizontal,
        "vertical" => Orientation::Vertical,
        _ => return None,
    })
}

/// The name for a [`Fit`].
pub const fn fit_name(fit: Fit) -> &'static str {
    match fit {
        Fit::Fill => "fill",
        Fit::Contain => "contain",
        Fit::Cover => "cover",
        Fit::Center => "center",
    }
}

/// The [`Fit`] a name stands for.
pub fn fit_from_name(name: &str) -> Option<Fit> {
    Some(match name {
        "fill" => Fit::Fill,
        "contain" => Fit::Contain,
        "cover" => Fit::Cover,
        "center" => Fit::Center,
        _ => return None,
    })
}

/// The name for a [`Presence`].
pub const fn presence_name(presence: Presence) -> &'static str {
    match presence {
        Presence::Online => "online",
        Presence::Offline => "offline",
        Presence::Busy => "busy",
    }
}

/// The [`Presence`] a name stands for.
pub fn presence_from_name(name: &str) -> Option<Presence> {
    Some(match name {
        "online" => Presence::Online,
        "offline" => Presence::Offline,
        "busy" => Presence::Busy,
        _ => return None,
    })
}

/// What a form file calls a [`Side`](crate::Side).
pub const fn side_name(side: crate::Side) -> &'static str {
    use crate::Side;
    match side {
        Side::Above => "above",
        Side::Below => "below",
        Side::Before => "before",
        Side::After => "after",
    }
}

/// The [`Side`](crate::Side) a name stands for.
pub fn side_from_name(name: &str) -> Option<crate::Side> {
    use crate::Side;
    Some(match name {
        "above" => Side::Above,
        "below" => Side::Below,
        "before" => Side::Before,
        "after" => Side::After,
        _ => return None,
    })
}

// -------------------------------------------------------------------- payload

/// What a widget hands its message constructor when it fires.
///
/// A `Button` holds an `M`. A `Checkbox` holds a `fn(bool) -> M`, a `List` a
/// `fn(usize) -> M`, a `Slider` a `fn(f32) -> M`. An engine resolving a name from
/// a file into the application's message type has to know which, so the
/// descriptor says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Payload {
    /// The widget holds the message itself: `M`.
    None,
    /// `fn(bool) -> M` — a checkbox, a toggle, a collapse.
    Bool,
    /// `fn(usize) -> M` — anything that selects one of several.
    Index,
    /// `fn(f32) -> M` — a slider, a rating.
    Number,
}

// --------------------------------------------------------------------- schema

/// What a property takes, and what an editor should offer for it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum PropertyKind {
    /// A string.
    Text,
    /// A checkbox.
    Bool,
    /// A whole number. The bounds are what an editor should offer; a widget that
    /// clamps still clamps.
    Int {
        /// Lowest sensible value.
        min: i32,
        /// Highest sensible value.
        max: i32,
    },
    /// A real number, likewise advisory.
    Float {
        /// Lowest sensible value.
        min: f32,
        /// Highest sensible value.
        max: f32,
    },
    /// One of a fixed set of names — a [`Role`], an [`Align`], a [`Fit`].
    Enum(&'static [&'static str]),
    /// A message name the application resolves, with the shape it must resolve
    /// to. Never settable here; see the [module docs](self).
    Message(Payload),
    /// A path relative to the form file. Never settable here; see the
    /// [module docs](self).
    Asset,
    /// A literal colour, written `#RRGGBB`.
    ///
    /// Carried as a [`Value::Text`], because that is what a form file holds and
    /// what an inspector's field edits; the kind is separate from `Text` so that
    /// an inspector knows to offer a swatch, and so that a widget rejecting
    /// `"chartreuse"` can say what it wanted.
    ///
    /// **The only widget with one is `video`**, whose ground is drawn behind a
    /// hardware plane and so is never composited with themed content. Everything
    /// else names a [`Role`] and lets the theme decide, which is what keeps a
    /// theme swap from leaving one widget the wrong colour.
    Color,
}

impl PropertyKind {
    /// A short name for this kind, for error messages.
    pub const fn noun(self) -> &'static str {
        match self {
            PropertyKind::Text => "a string",
            PropertyKind::Bool => "true or false",
            PropertyKind::Int { .. } => "a whole number",
            PropertyKind::Float { .. } => "a number",
            PropertyKind::Enum(_) => "one of the listed names",
            PropertyKind::Message(_) => "a message name",
            PropertyKind::Asset => "a path",
            PropertyKind::Color => "a colour like #RRGGBB",
        }
    }
}

/// One setting a widget has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Property {
    /// The name a form file and an inspector use. Kebab-case.
    pub name: &'static str,
    /// What it takes.
    pub kind: PropertyKind,
    /// One line, shown as a tooltip in the inspector and rendered into the
    /// widget's documentation.
    pub doc: &'static str,
    /// Whether the number is a **length in logical pixels**, and so multiplies
    /// with the scale factor.
    ///
    /// A widget's numbers are not all the same kind of thing. A `Label`'s `size`
    /// is 16 logical pixels and is 32 at 2×; a `Carousel`'s `auto-advance-ms` is
    /// 4000 milliseconds and is 4000 at every scale; a `List`'s `selected` is
    /// the third row and is the third row on a wall. Only the widget knows
    /// which of its own numbers are lengths, so only the widget can say — the
    /// same reason the rest of this descriptor exists rather than a table
    /// somewhere central.
    ///
    /// [`Form::build_scaled`] is what reads it.
    ///
    /// [`Form::build_scaled`]: https://docs.rs/denise-forms/latest/denise_forms/struct.Form.html#method.build_scaled
    pub pixels: bool,
}

impl Property {
    /// A property. Not a length unless [`in_pixels`](Property::in_pixels) says so.
    pub const fn new(name: &'static str, kind: PropertyKind, doc: &'static str) -> Self {
        Self {
            name,
            kind,
            doc,
            pixels: false,
        }
    }

    /// This property is a length in logical pixels.
    ///
    /// Say it about a number that should be twice as many at 2× — a text size, a
    /// row height, a border width — and not about a count, a duration, an index
    /// or a proportion. See [`Property::pixels`].
    ///
    /// ```
    /// # use denise_ui::widgets::{Property, PropertyKind};
    /// const SIZE: Property = Property::new(
    ///     "size",
    ///     PropertyKind::Int { min: 6, max: 96 },
    ///     "Text size in logical pixels.",
    /// )
    /// .in_pixels();
    ///
    /// assert!(SIZE.pixels);
    /// assert!(!Property::new("selected", PropertyKind::Int { min: 0, max: 99 }, "").pixels);
    /// ```
    #[must_use]
    pub const fn in_pixels(mut self) -> Self {
        self.pixels = true;
        self
    }

    /// Whether [`Describe::set`] can apply this property.
    ///
    /// False for a message and for an asset, which the engine supplies at
    /// construction because this crate can hold neither. See the
    /// [module docs](self).
    pub const fn is_settable(&self) -> bool {
        !matches!(self.kind, PropertyKind::Message(_) | PropertyKind::Asset)
    }
}

// ---------------------------------------------------------------------- value

/// A property's value, owned and untyped.
///
/// The bridge between a string in a file and a typed call on a widget. Small on
/// purpose: everything a form can say is one of these.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Value {
    /// [`PropertyKind::Text`].
    Text(String),
    /// [`PropertyKind::Bool`].
    Bool(bool),
    /// [`PropertyKind::Int`].
    Int(i32),
    /// [`PropertyKind::Float`].
    Float(f32),
    /// [`PropertyKind::Enum`] — always one of the names in the property's table,
    /// which is why it is `'static`: the caller has already found it there.
    Enum(&'static str),
}

impl Value {
    /// A text value.
    pub fn text(text: impl Into<String>) -> Self {
        Value::Text(text.into())
    }

    /// The name of a [`Role`].
    pub const fn role(role: Role) -> Self {
        Value::Enum(role_name(role))
    }

    /// The string, or a mismatch.
    pub fn as_text(self) -> Result<String, Mismatch> {
        match self {
            Value::Text(text) => Ok(text),
            _ => Err(Mismatch::wrong(PropertyKind::Text)),
        }
    }

    /// The boolean, or a mismatch.
    pub fn as_bool(self) -> Result<bool, Mismatch> {
        match self {
            Value::Bool(value) => Ok(value),
            _ => Err(Mismatch::wrong(PropertyKind::Bool)),
        }
    }

    /// The whole number, or a mismatch.
    pub fn as_int(self) -> Result<i32, Mismatch> {
        match self {
            Value::Int(value) => Ok(value),
            _ => Err(Mismatch::wrong(PropertyKind::Int {
                min: i32::MIN,
                max: i32::MAX,
            })),
        }
    }

    /// The number, or a mismatch. A whole number is accepted, because a form file
    /// writes `value=1` for a float as readily as `value=1.0`.
    pub fn as_float(self) -> Result<f32, Mismatch> {
        match self {
            Value::Float(value) => Ok(value),
            Value::Int(value) => Ok(value as f32),
            _ => Err(Mismatch::wrong(PropertyKind::Float {
                min: f32::MIN,
                max: f32::MAX,
            })),
        }
    }

    /// The name, or a mismatch.
    pub fn as_name(self) -> Result<&'static str, Mismatch> {
        match self {
            Value::Enum(name) => Ok(name),
            _ => Err(Mismatch::wrong(PropertyKind::Enum(&[]))),
        }
    }

    /// A whole number narrowed to a text size, clamped rather than wrapped.
    pub fn as_size(self) -> Result<u16, Mismatch> {
        Ok(self.as_int()?.clamp(1, u16::MAX as i32) as u16)
    }

    /// A whole number narrowed to a count, clamped at zero.
    pub fn as_count(self) -> Result<u32, Mismatch> {
        Ok(self.as_int()?.max(0) as u32)
    }

    /// A whole number narrowed to a duration in milliseconds.
    pub fn as_millis(self) -> Result<u64, Mismatch> {
        Ok(self.as_int()?.max(0) as u64)
    }

    /// A whole number narrowed to an index, clamped at zero.
    pub fn as_index(self) -> Result<usize, Mismatch> {
        Ok(self.as_int()?.max(0) as usize)
    }

    /// The name of an [`Align`].
    pub const fn align(align: Align) -> Self {
        Value::Enum(align_name(align))
    }

    /// The name of a [`Radius`].
    pub const fn radius(radius: Radius) -> Self {
        Value::Enum(radius_name(radius))
    }

    /// The name of an [`Orientation`].
    pub const fn orientation(orientation: Orientation) -> Self {
        Value::Enum(orientation_name(orientation))
    }

    /// The name of a [`Fit`].
    pub const fn fit(fit: Fit) -> Self {
        Value::Enum(fit_name(fit))
    }

    /// The name of a [`Presence`].
    pub const fn presence(presence: Presence) -> Self {
        Value::Enum(presence_name(presence))
    }

    /// The [`Role`] this name stands for, or a mismatch.
    pub fn as_role(self) -> Result<Role, Mismatch> {
        role_from_name(self.as_name()?).ok_or_else(|| Mismatch::wrong(PropertyKind::Enum(ROLES)))
    }

    /// The [`Align`] this name stands for, or a mismatch.
    pub fn as_align(self) -> Result<Align, Mismatch> {
        align_from_name(self.as_name()?)
            .ok_or_else(|| Mismatch::wrong(PropertyKind::Enum(ALIGNMENTS)))
    }

    /// The [`Radius`] this name stands for, or a mismatch.
    pub fn as_radius(self) -> Result<Radius, Mismatch> {
        radius_from_name(self.as_name()?).ok_or_else(|| Mismatch::wrong(PropertyKind::Enum(RADII)))
    }

    /// The [`Orientation`] this name stands for, or a mismatch.
    pub fn as_orientation(self) -> Result<Orientation, Mismatch> {
        orientation_from_name(self.as_name()?)
            .ok_or_else(|| Mismatch::wrong(PropertyKind::Enum(ORIENTATIONS)))
    }

    /// The [`Fit`] this name stands for, or a mismatch.
    pub fn as_fit(self) -> Result<Fit, Mismatch> {
        fit_from_name(self.as_name()?).ok_or_else(|| Mismatch::wrong(PropertyKind::Enum(FITS)))
    }

    /// The [`Presence`] this name stands for, or a mismatch.
    pub fn as_presence(self) -> Result<Presence, Mismatch> {
        presence_from_name(self.as_name()?)
            .ok_or_else(|| Mismatch::wrong(PropertyKind::Enum(PRESENCES)))
    }
}

// --------------------------------------------------------------------- errors

/// Why a widget would not take a value, without saying which widget.
///
/// [`Describe::apply`] returns this and [`Describe::set`] turns it into a
/// [`PropertyError`] that names the widget and the property. The split exists so
/// that twenty-six `apply` implementations do not each repeat the context they
/// all share.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mismatch {
    /// No such property on this widget.
    Unknown,
    /// The property exists; the value was the wrong shape.
    WrongType {
        /// What the property takes.
        expected: PropertyKind,
    },
    /// The property exists and the widget cannot hold it — a message needs the
    /// application's type, an asset needs a loader. See the [module docs](self).
    Supplied,
}

impl Mismatch {
    const fn wrong(expected: PropertyKind) -> Self {
        Mismatch::WrongType { expected }
    }
}

/// A property that could not be set, and everything needed to say so usefully.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyError {
    /// The widget's kind, as a form file spells it.
    pub kind: &'static str,
    /// The property that was asked for.
    pub name: String,
    /// What went wrong.
    pub mismatch: Mismatch,
    /// Everything this widget does accept, for the "expected one of" line.
    pub accepted: &'static [Property],
}

impl fmt::Display for PropertyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.mismatch {
            Mismatch::Unknown => {
                write!(f, "`{}` has no property `{}`", self.kind, self.name)?;
                if !self.accepted.is_empty() {
                    let names: Vec<&str> = self.accepted.iter().map(|p| p.name).collect();
                    write!(f, "; it accepts {}", names.join(", "))?;
                }
                Ok(())
            }
            Mismatch::WrongType { expected } => write!(
                f,
                "`{}` on `{}` takes {}",
                self.name,
                self.kind,
                expected.noun()
            ),
            Mismatch::Supplied => write!(
                f,
                "`{}` on `{}` is supplied when the widget is built, not set afterwards",
                self.name, self.kind
            ),
        }
    }
}

impl core::error::Error for PropertyError {}

// ------------------------------------------------------------------- describe

/// A widget that knows its own properties.
///
/// Implemented beside each widget. See the [module docs](self) for why the list
/// lives here rather than in whatever reads it.
pub trait Describe {
    /// The name a form file uses for this widget. Kebab-case.
    const KIND: &'static str;

    /// One line saying what this widget **is**, for somebody choosing one.
    ///
    /// A designer's palette shows it as a tooltip, which is the difference
    /// between twenty-five bare names and a catalogue. Not a description of the
    /// API and not a sentence about this type — a sentence about the thing on
    /// screen, in the words of a person deciding whether they want it.
    ///
    /// Deliberately required rather than defaulted: adding a widget without one
    /// should not compile, because a widget nobody can identify in the palette
    /// is a widget nobody reaches for.
    const DOC: &'static str;

    /// Which shelf of the catalogue this belongs on.
    const GROUP: Group;

    /// Every property, in the order an inspector should show them.
    const PROPERTIES: &'static [Property];

    /// The current value.
    ///
    /// `None` for three different situations, which the caller tells apart by
    /// consulting [`PROPERTIES`](Describe::PROPERTIES): a property this widget
    /// does not have, one it cannot report (a message or an asset — see the
    /// [module docs](self)), and one that is simply not set, such as the
    /// selection of a `Select` with nothing selected. The third is what makes
    /// "a property at its default is not written to the file" implementable:
    /// nothing to report, nothing to write.
    fn get(&self, name: &str) -> Option<Value>;

    /// Applies a value, reporting only what went wrong.
    ///
    /// Implement this one. Call [`Describe::set`], which adds the widget's name,
    /// the property's name and the list of what would have been accepted.
    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch>;

    /// Applies a value, reporting what went wrong and where.
    fn set(&mut self, name: &str, value: Value) -> Result<(), PropertyError> {
        self.apply(name, value).map_err(|mismatch| PropertyError {
            kind: Self::KIND,
            name: name.to_string(),
            mismatch,
            accepted: Self::PROPERTIES,
        })
    }
}

/// [`Describe`], reachable through a `dyn Widget<M>`.
///
/// [`Describe`] has associated constants, so it is not object-safe, and the tree
/// stores widgets boxed. This is the same four questions asked of a trait object,
/// blanket-implemented for everything that describes itself — never implement it
/// by hand.
///
/// [`Ui::set_property`](crate::Ui::set_property) is what calls it.
pub trait DynDescribe {
    /// See [`Describe::KIND`].
    fn kind(&self) -> &'static str;
    /// See [`Describe::PROPERTIES`].
    fn properties(&self) -> &'static [Property];
    /// See [`Describe::get`].
    fn get_property(&self, name: &str) -> Option<Value>;
    /// See [`Describe::set`].
    fn set_property(&mut self, name: &str, value: Value) -> Result<(), PropertyError>;
}

impl<T: Describe> DynDescribe for T {
    fn kind(&self) -> &'static str {
        T::KIND
    }
    fn properties(&self) -> &'static [Property] {
        T::PROPERTIES
    }
    fn get_property(&self, name: &str) -> Option<Value> {
        self.get(name)
    }
    fn set_property(&mut self, name: &str, value: Value) -> Result<(), PropertyError> {
        self.set(name, value)
    }
}

// ------------------------------------------------------------------- registry

/// One widget in the catalogue [`all`] returns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidgetInfo {
    /// The name a form file uses.
    pub kind: &'static str,
    /// One line saying what it is. See [`Describe::DOC`].
    pub doc: &'static str,
    /// Which shelf of the catalogue it belongs on.
    pub group: Group,
    /// What it accepts.
    pub properties: &'static [Property],
}

impl WidgetInfo {
    /// The entry for a widget.
    pub const fn of<W: Describe>() -> Self {
        Self {
            kind: W::KIND,
            doc: W::DOC,
            group: W::GROUP,
            properties: W::PROPERTIES,
        }
    }

    /// The property of this name, if it has one.
    pub fn property(&self, name: &str) -> Option<&'static Property> {
        self.properties.iter().find(|p| p.name == name)
    }
}

/// The shelves a catalogue of widgets is arranged on.
///
/// Six, and deliberately few: a palette that has to be *read* to be searched has
/// failed, and the point of grouping twenty-five rows is that the eye lands on
/// the right handful. The order here is the order a palette should show them,
/// which is roughly how often somebody reaches for one.
///
/// A widget declares its own through [`Describe::GROUP`], for the same reason it
/// declares its own properties: there is no table of widgets anywhere in this
/// workspace and this is not the place to start one.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Group {
    /// Something a person operates: it takes a message and emits one.
    Input,
    /// Something a person reads. Text, and the decorations around text.
    Display,
    /// Something that says how far along, how busy, or how much.
    Indicator,
    /// Something other widgets go inside.
    Container,
    /// Rows and columns of content, with a selection.
    Data,
    /// Pictures and video.
    Media,
}

impl Group {
    /// Every one, in the order a palette shows them.
    pub const ALL: [Self; 6] = [
        Self::Input,
        Self::Display,
        Self::Indicator,
        Self::Container,
        Self::Data,
        Self::Media,
    ];

    /// The heading a palette writes above the shelf.
    ///
    /// ```
    /// # use denise_ui::widgets::Group;
    /// assert_eq!(Group::Input.name(), "input");
    /// // Every group has one, and no two share it.
    /// let mut names: Vec<&str> = Group::ALL.iter().map(|g| g.name()).collect();
    /// names.sort_unstable();
    /// names.dedup();
    /// assert_eq!(names.len(), Group::ALL.len());
    /// ```
    pub const fn name(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Display => "display",
            Self::Indicator => "indicator",
            Self::Container => "container",
            Self::Data => "data",
            Self::Media => "media",
        }
    }
}

/// Every widget that ships with this crate.
///
/// A palette lists these rather than naming widgets itself, so the twenty-sixth
/// widget appears in the designer without the designer changing. A test asserts
/// that this and [`widgets`](super) hold the same set, so joining it is not
/// something a new widget can be merged without.
pub fn all() -> &'static [WidgetInfo] {
    ALL
}

/// The catalogue [`all`] returns.
///
/// `Void` stands in for the message type, which none of the descriptions depend
/// on.
static ALL: &[WidgetInfo] = &[
    WidgetInfo::of::<super::Alert>(),
    WidgetInfo::of::<super::Avatar>(),
    WidgetInfo::of::<super::Badge>(),
    WidgetInfo::of::<super::Button<crate::Void>>(),
    WidgetInfo::of::<super::Carousel<crate::Void>>(),
    WidgetInfo::of::<super::Checkbox<crate::Void>>(),
    WidgetInfo::of::<super::Collapse<crate::Void>>(),
    WidgetInfo::of::<super::Divider>(),
    WidgetInfo::of::<super::Image>(),
    WidgetInfo::of::<super::Label>(),
    WidgetInfo::of::<super::List<crate::Void>>(),
    WidgetInfo::of::<super::Panel>(),
    WidgetInfo::of::<super::Progress>(),
    WidgetInfo::of::<super::RadialProgress>(),
    WidgetInfo::of::<super::RadioGroup<crate::Void>>(),
    WidgetInfo::of::<super::Rating<crate::Void>>(),
    WidgetInfo::of::<super::Select<crate::Void>>(),
    WidgetInfo::of::<super::Slider<crate::Void>>(),
    WidgetInfo::of::<super::Spinner>(),
    WidgetInfo::of::<super::Table<crate::Void>>(),
    WidgetInfo::of::<super::Tabs<crate::Void>>(),
    WidgetInfo::of::<super::TextInput<crate::Void>>(),
    WidgetInfo::of::<super::Timeline>(),
    WidgetInfo::of::<super::Toggle<crate::Void>>(),
    WidgetInfo::of::<super::Video>(),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_role_round_trips_through_its_name() {
        for (index, name) in ROLES.iter().enumerate() {
            let role = role_from_name(name).expect("a name in the table names a role");
            assert_eq!(role as usize, index, "{name} is out of order");
            assert_eq!(role_name(role), *name);
        }
    }

    #[test]
    fn a_name_outside_the_table_is_not_a_role() {
        assert_eq!(role_from_name("puce"), None);
        assert_eq!(role_from_name(""), None);
        assert_eq!(role_from_name("primary-"), None);
    }

    #[test]
    fn the_catalogue_names_are_unique_and_sorted() {
        let mut names: Vec<&str> = all().iter().map(|w| w.kind).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "two widgets share a kind");
    }

    #[test]
    fn no_widget_declares_the_same_property_twice() {
        for widget in all() {
            let mut names: Vec<&str> = widget.properties.iter().map(|p| p.name).collect();
            let count = names.len();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), count, "{} repeats a property", widget.kind);
        }
    }

    #[test]
    fn every_property_is_kebab_case_and_documented() {
        for widget in all() {
            for property in widget.properties {
                assert!(
                    !property.name.is_empty()
                        && property
                            .name
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                    "{}.{} is not kebab-case",
                    widget.kind,
                    property.name
                );
                assert!(
                    !property.doc.is_empty(),
                    "{}.{} has no documentation",
                    widget.kind,
                    property.name
                );
            }
        }
    }

    #[test]
    fn every_widget_says_in_one_line_what_it_is() {
        for widget in all() {
            let doc = widget.doc;
            assert!(!doc.is_empty(), "{} says nothing about itself", widget.kind);
            // One line, because it is a tooltip.
            assert!(
                !doc.contains('\n'),
                "{}'s line is more than one",
                widget.kind
            );
            // A sentence a person reads, not a fragment: a capital and a stop.
            assert!(
                doc.starts_with(|c: char| c.is_uppercase()),
                "{}: `{doc}` does not start a sentence",
                widget.kind,
            );
            assert!(
                doc.ends_with('.'),
                "{}: `{doc}` does not end one",
                widget.kind
            );
            // Long enough to say something, short enough to read at a glance.
            assert!(
                (20..=100).contains(&doc.len()),
                "{}: `{doc}` is {} characters",
                widget.kind,
                doc.len(),
            );
        }
    }

    #[test]
    fn no_two_widgets_describe_themselves_the_same_way() {
        // Two identical lines means one of them is wrong: the whole point is
        // telling a `checkbox` from a `toggle` while choosing between them.
        let mut docs: Vec<&str> = all().iter().map(|w| w.doc).collect();
        let count = docs.len();
        docs.sort_unstable();
        docs.dedup();
        assert_eq!(docs.len(), count, "two widgets say the same thing");
    }

    #[test]
    fn every_group_has_something_on_it() {
        // A shelf with nothing on it is a heading a palette would draw over
        // nothing, and a sign that the set of groups drifted from the widgets.
        for group in Group::ALL {
            assert!(
                all().iter().any(|w| w.group == group),
                "nothing is `{}`",
                group.name(),
            );
        }
        // And every widget is on one of them, which the type already promises;
        // this catches a group added to the enum and left out of `ALL`.
        for widget in all() {
            assert!(
                Group::ALL.contains(&widget.group),
                "{} is in a group `Group::ALL` does not list",
                widget.kind,
            );
        }
    }

    #[test]
    fn an_enum_property_offers_names_it_would_accept() {
        for widget in all() {
            for property in widget.properties {
                if let PropertyKind::Enum(names) = property.kind {
                    assert!(
                        !names.is_empty(),
                        "{}.{} offers no names",
                        widget.kind,
                        property.name
                    );
                }
            }
        }
    }
}
