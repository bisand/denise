//! Static text.

use alloc::string::{String, ToString};

use denise::Role;
use denise_render::Canvas;
use denise_text::TextStyle;

use crate::widget::{PaintCtx, Widget};
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
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        let color = ctx.theme.color(self.role);
        draw_aligned(
            canvas, ctx.text, self.style, ctx.bounds, self.align, &self.text, color,
        );
    }
}
