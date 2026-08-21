//! The rectangle a video plane sits in.

use denise::Color;
use denise_render::Canvas;

use crate::widget::{PaintCtx, Widget};

/// Where a video goes: a placeholder that reserves space in the tree and
/// paints the letterbox ground. **The video itself never comes through
/// here** — a hardware-decoded frame is scanned out on a DRM *plane* the
/// display controller composites, and the frame never touches the rasteriser.
///
/// The widget's whole job is to be a rectangle the layout owns:
///
/// ```
/// # use denise::{Rect, Size, theme};
/// # use denise_ui::{Ui, widgets::Panel};
/// # #[derive(Clone, Debug)] enum Msg { Noop }
/// # fn demo() -> Option<()> {
/// # let mut ui: Ui<Msg> = Ui::new(Size::new(1920, 1080), theme::DARK);
/// # let root = ui.root();
/// # use denise_ui::widgets::Video;
/// # struct Player;                       // `denise-video`'s, in a real application
/// # impl Player { fn set_dst(&self, _: Rect) {} }
/// # let player = Player;
/// let video = ui.add(root, Video::new(), Rect::new(40, 40, 640, 360))?;
/// // hand the rectangle to the plane, and again whenever layout changes:
/// player.set_dst(ui.bounds(video).unwrap_or(Rect::ZERO));
/// # Some(()) }
/// ```
///
/// `denise-video`'s `Player` does the rest, against the same DRM card the
/// surface owns. On desktop backends — a window on macOS, a preview under
/// winit — there is no plane and the placeholder is simply what shows, which
/// is the honest degradation: layout stays right everywhere, pixels move only
/// where the hardware exists.
///
/// Not interactive and not focusable, like [`Label`](super::Label): a video
/// inside a clickable card must not swallow the click. Play, stop and loop
/// are the application talking to the player, not messages through the tree —
/// the tree does not own the transport, so it does not pretend to.
#[derive(Clone, Debug)]
pub struct Video {
    ground: Color,
}

impl Video {
    /// A placeholder painting true black — video letterboxing is black
    /// everywhere else a person has watched anything.
    pub fn new() -> Self {
        Self {
            ground: Color::rgb(0, 0, 0),
        }
    }

    /// Overrides the ground colour, for a panel whose design letterboxes in
    /// something other than black.
    pub fn with_ground(mut self, ground: Color) -> Self {
        self.ground = ground;
        self
    }
}

impl Default for Video {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: 'static> Widget<M> for Video {
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        canvas.fill_rect(ctx.bounds, self.ground);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_video_placeholder_takes_no_input() {
        let v = Video::new();
        assert!(!Widget::<usize>::focusable(&v));
        assert!(!Widget::<usize>::accepts_pointer(&v));
    }
}
