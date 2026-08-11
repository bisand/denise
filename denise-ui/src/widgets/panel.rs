//! A themed rectangle: the background every other widget sits on.

use denise::{Radius, Role};
use denise_render::Canvas;

use crate::widget::{PaintCtx, Widget};

/// A filled, optionally bordered rounded rectangle.
///
/// Panels are not interactive and are invisible to hit testing, so putting a
/// button on one does not mean the panel steals the click.
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
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            fill: Some(Role::Base200),
            border: Some(Role::Base300),
            border_width: 1,
            radius: Radius::Box,
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
        }
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
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
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
