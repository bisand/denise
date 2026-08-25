//! Static text.

use alloc::string::{String, ToString};

use denise::Role;
use denise_render::Canvas;
use denise_text::TextStyle;

use crate::widget::{PaintCtx, Widget};
use crate::widgets::describe::{
    ALIGNMENTS, Describe, DynDescribe, Group, Mismatch, Property, PropertyKind, ROLES, Value,
};
use crate::widgets::style::{Align, draw_aligned};

/// A run of text drawn in a content colour, aligned inside its bounds.
///
/// Not interactive and not focusable, so a label inside a button never intercepts
/// the click.
#[derive(Clone, Debug)]
pub struct Label {
    text: String,
    role: Role,
    align: (Align, Align),
    style: TextStyle,
}

impl Label {
    /// A label in [`Role::BaseContent`], left-aligned and vertically centred, in
    /// the built-in font at 16 px.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            role: Role::BaseContent,
            align: (Align::Start, Align::Center),
            style: TextStyle::built_in(16),
        }
    }

    /// Sets the colour role. Pass a `*Content` role, or a surface role to draw the
    /// label *in* that colour rather than on it.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Sets horizontal and vertical alignment.
    pub fn with_align(mut self, horizontal: Align, vertical: Align) -> Self {
        self.align = (horizontal, vertical);
        self
    }

    /// Sets the font and size.
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets the size, keeping the font.
    pub fn with_size(mut self, size_px: u16) -> Self {
        self.style.size_px = size_px;
        self
    }

    /// The font and size this label draws in.
    #[inline]
    pub const fn style(&self) -> TextStyle {
        self.style
    }

    /// The current text.
    #[inline]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Replaces the text.
    ///
    /// Reach this through [`Ui::widget_mut`](crate::Ui::widget_mut), which marks
    /// the node dirty on the way in.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    /// Replaces the font and size.
    ///
    /// For an application that registers a font after building its tree, which is
    /// the ordinary case: the tree has to exist before anyone knows whether the
    /// font file was there.
    pub fn set_style(&mut self, style: TextStyle) {
        self.style = style;
    }

    /// Replaces the colour role.
    pub fn set_role(&mut self, role: Role) {
        self.role = role;
    }

    /// Replaces the text only if it differs, reporting whether it changed.
    ///
    /// For the common case of writing a reading into a label every tick: an
    /// unchanged value should not cost a repaint, and `widget_mut` cannot know
    /// that on its own.
    pub fn update(&mut self, text: &str) -> bool {
        let changed = self.text != text;
        if changed {
            self.text = text.to_string();
        }
        changed
    }
}

impl<M: 'static> Widget<M> for Label {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        let color = ctx.theme.color(self.role);
        draw_aligned(
            canvas, ctx.text, self.style, ctx.bounds, self.align, &self.text, color,
        );
    }
}

impl Describe for Label {
    const KIND: &'static str = "label";
    const DOC: &'static str = "Text that is read and not touched.";
    const GROUP: Group = Group::Display;

    const PROPERTIES: &'static [Property] = &[
        Property::new("text", PropertyKind::Text, "The text drawn."),
        Property::new(
            "role",
            PropertyKind::Enum(ROLES),
            "Colour role. A `*Content` role draws on a surface; a surface role draws the text in that colour.",
        ),
        Property::new(
            "align",
            PropertyKind::Enum(ALIGNMENTS),
            "Where the text sits horizontally in its box.",
        ),
        Property::new(
            "valign",
            PropertyKind::Enum(ALIGNMENTS),
            "Where the text sits vertically in its box.",
        ),
        Property::new(
            "size",
            PropertyKind::Int { min: 6, max: 96 },
            "Text size in logical pixels.",
        )
        .in_pixels(),
    ];

    fn get(&self, name: &str) -> Option<Value> {
        Some(match name {
            "text" => Value::text(self.text.as_str()),
            "role" => Value::role(self.role),
            "align" => Value::align(self.align.0),
            "valign" => Value::align(self.align.1),
            "size" => Value::Int(i32::from(self.style.size_px)),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            "text" => self.text = value.as_text()?,
            "role" => self.role = value.as_role()?,
            "align" => self.align.0 = value.as_align()?,
            "valign" => self.align.1 = value.as_align()?,
            "size" => self.style.size_px = value.as_size()?,
            _ => return Err(Mismatch::Unknown),
        }
        Ok(())
    }
}
