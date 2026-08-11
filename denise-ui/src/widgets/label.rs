//! Static text.

use alloc::string::{String, ToString};

use denise::Role;
use denise_render::Canvas;
use denise_render::font::{self, BitmapFont};

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
    horizontal: Align,
    vertical: Align,
    scale: i32,
}

impl Label {
    /// A label in [`Role::BaseContent`], left-aligned and vertically centred.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            role: Role::BaseContent,
            horizontal: Align::Start,
            vertical: Align::Center,
            scale: 2,
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
        self.horizontal = horizontal;
        self.vertical = vertical;
        self
    }

    /// Sets the integer glyph scale. `2` is the default; `3` or `4` suits a panel
    /// read from across a room.
    pub fn with_scale(mut self, scale: i32) -> Self {
        self.scale = scale.max(1);
        self
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

    /// Height one line of this label occupies.
    #[inline]
    pub const fn line_height(&self) -> i32 {
        font::CELL_HEIGHT * self.scale
    }

    /// Width this label's text needs.
    pub fn preferred_width(&self) -> i32 {
        BitmapFont::measure(&font::BUILT_IN, &self.text, self.scale).width as i32
    }
}

impl<M: 'static> Widget<M> for Label {
    fn paint(&self, ctx: &PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        draw_aligned(
            canvas,
            ctx.bounds,
            self.horizontal,
            self.vertical,
            self.scale,
            &self.text,
            ctx.theme.color(self.role),
        );
    }
}
