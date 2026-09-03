//! A themed rectangle: the background every other widget sits on.

use denise::Pen;
use denise::{Radius, Role};

use crate::widget::{PaintCtx, Widget};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Property, PropertyKind, RADII, ROLES, Value,
    role_from_name,
};

/// A filled, optionally bordered rounded rectangle.
///
/// Panels are not interactive and are invisible to hit testing, so putting a
/// button on one does not mean the panel steals the click. [`Panel::backdrop`]
/// is the exception, for the sheet under an overlay's contents.
#[derive(Clone, Copy, Debug)]
pub struct Panel {
    /// Background role, or `None` to leave what is underneath alone.
    pub fill: Option<Role>,
    /// Border role, or `None` for no border.
    pub border: Option<Role>,
    /// Border thickness in pixels, drawn inside the bounds.
    pub border_width: i32,
    /// Corner rounding token. The theme decides the pixels.
    pub radius: Radius,
    /// Whether presses stop here instead of falling through.
    ///
    /// See [`Panel::backdrop`]. Off for every ordinary panel.
    pub backdrop: bool,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            fill: Some(Role::Base200),
            border: Some(Role::Base300),
            border_width: 1,
            radius: Radius::Box,
            backdrop: false,
        }
    }
}

impl Panel {
    /// A panel filled with `role` and no border.
    pub const fn filled(role: Role) -> Self {
        Self {
            fill: Some(role),
            border: None,
            border_width: 0,
            radius: Radius::Box,
            backdrop: false,
        }
    }

    /// A panel that draws nothing: no fill, no border.
    ///
    /// A container, for when the *grouping* is the point — a `tabs` node's
    /// pages, one per tab, shown and hidden as a unit. The tree already gives a
    /// node a rectangle, a clip and children; this is the widget for a node
    /// that wants those and no appearance of its own.
    ///
    /// ```
    /// # use denise_ui::widgets::Panel;
    /// // Nothing to see, and that is the whole idea.
    /// let page = Panel::bare();
    /// # let _ = page;
    /// ```
    pub const fn bare() -> Self {
        Self {
            fill: None,
            border: None,
            border_width: 0,
            radius: Radius::Box,
            backdrop: false,
        }
    }

    /// A panel that presses stop at, without disturbing the focus.
    ///
    /// The sheet behind an overlay's contents. An ordinary panel is invisible to
    /// hit testing, which is right for a card with a button on it and wrong for
    /// the sheet under an on-screen keyboard: a finger landing in the gap
    /// between two keys falls through to whatever is behind the overlay, and
    /// pressing *that* takes the focus away from the field being typed into —
    /// so a near-miss dismisses the keyboard.
    ///
    /// This absorbs the press and leaves the focus exactly where it was, which
    /// is the same bargain [`Button::no_focus`](crate::widgets::Button::no_focus)
    /// makes for the keys themselves.
    #[must_use]
    pub const fn backdrop(mut self) -> Self {
        self.backdrop = true;
        self
    }

    /// Sets the corner rounding token.
    pub const fn with_radius(mut self, radius: Radius) -> Self {
        self.radius = radius;
        self
    }

    /// Sets the border role and thickness.
    pub const fn with_border(mut self, role: Role, width: i32) -> Self {
        self.border = Some(role);
        self.border_width = width;
        self
    }
}

impl<M: 'static> Widget<M> for Panel {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn accepts_pointer(&self) -> bool {
        self.backdrop
    }

    /// A backdrop is pressed *past*, not pressed: it must not move the focus and
    /// must not clear it.
    fn preserves_focus(&self) -> bool {
        self.backdrop
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        let radius = ctx.theme.radius(self.radius);
        if let Some(role) = self.fill {
            canvas.fill_rounded_rect(ctx.bounds, radius, ctx.theme.color(role));
        }
        if let Some(role) = self.border
            && self.border_width > 0
        {
            canvas.stroke_rounded_rect(
                ctx.bounds,
                radius,
                self.border_width,
                ctx.theme.color(role),
            );
        }
    }
}

/// The name that clears a colour rather than choosing one.
const NONE: &str = "none";

/// Every [`Role`], plus [`NONE`].
///
/// `fill` and `border` are `Option<Role>`, so a form file needs a way to say the
/// absence — `fill=none` — and an inspector needs to offer it in the same list it
/// offers the colours in. Built from [`ROLES`] rather than written out again
/// because a slice cannot be concatenated in a `const`, and a second copy of the
/// role names is a second thing to forget when one is added.
const fn roles_or_none() -> [&'static str; ROLES.len() + 1] {
    let mut names = [NONE; ROLES.len() + 1];
    let mut i = 0;
    while i < ROLES.len() {
        names[i] = ROLES[i];
        i += 1;
    }
    // The last stays `NONE`, which is what the array was filled with.
    names
}

/// [`roles_or_none`] as a slice, which is what [`PropertyKind::Enum`] takes.
const ROLES_OR_NONE: &[&str] = &roles_or_none();

/// A role, or the absence of one.
fn role_or_none(value: Value) -> Result<Option<Role>, Mismatch> {
    let name = value.as_name()?;
    if name == NONE {
        return Ok(None);
    }
    role_from_name(name).map(Some).ok_or(Mismatch::WrongType {
        expected: PropertyKind::Enum(ROLES_OR_NONE),
    })
}

impl Describe for Panel {
    const KIND: &'static str = "panel";
    const DOC: &'static str = "A themed rectangle: the background other widgets sit on.";
    const GROUP: Group = Group::Container;
    const ICON: &'static denise::icon::Icon = &super::icons::PANEL;

    const PROPERTIES: &'static [Property] = &[
        Property::new(
            "fill",
            PropertyKind::Enum(ROLES_OR_NONE),
            "Surface colour. `none` leaves what is underneath alone.",
        ),
        Property::new(
            "border",
            PropertyKind::Enum(ROLES_OR_NONE),
            "Border colour. `none` for no border.",
        ),
        Property::new(
            "border-width",
            PropertyKind::Int { min: 0, max: 16 },
            "Border thickness in pixels, drawn inside the bounds.",
        )
        .in_pixels(),
        Property::new(
            "radius",
            PropertyKind::Enum(RADII),
            "Corner rounding token. The theme decides the pixels.",
        ),
        Property::new(
            "backdrop",
            PropertyKind::Bool,
            "This panel absorbs presses rather than letting them fall through, and leaves the focus where it is. What the sheet under an on-screen keyboard is.",
        ),
    ];

    fn get(&self, name: &str) -> Option<Value> {
        Some(match name {
            // A panel with no fill reports nothing rather than reporting
            // `none`: an unset property is one nothing has to write down.
            "fill" => Value::role(self.fill?),
            "border" => Value::role(self.border?),
            "border-width" => Value::Int(self.border_width),
            "radius" => Value::radius(self.radius),
            "backdrop" => Value::Bool(self.backdrop),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            "fill" => self.fill = role_or_none(value)?,
            "border" => self.border = role_or_none(value)?,
            // Negative is not a thinner border, it is an inverted rectangle by
            // the time `stroke_rounded_rect` sees it.
            "border-width" => self.border_width = value.as_int()?.max(0),
            "radius" => self.radius = value.as_radius()?,
            "backdrop" => self.backdrop = value.as_bool()?,
            _ => return Err(Mismatch::Unknown),
        }
        Ok(())
    }
}
