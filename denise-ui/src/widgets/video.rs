//! The rectangle a video plane sits in.

use alloc::format;

use denise::Color;
use denise_render::Canvas;

use crate::widget::{PaintCtx, Widget};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Property, PropertyKind, Value,
};

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
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        canvas.fill_rect(ctx.bounds, self.ground);
    }
}

/// The colour `"#RRGGBB"` names, opaque.
///
/// The one literal colour in the format, so this is the only hand-written
/// colour parser: everything else names a [`Role`](denise::Role) and the theme
/// decides. Exactly six hex digits, with the `#` optional — a shorter form
/// would have to guess whether `#abc` is a shorthand or a typo, and the ground
/// of a video plane is not worth the guess.
fn ground_from_hex(text: &str) -> Option<Color> {
    let digits = text.strip_prefix('#').unwrap_or(text);
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok().map(Color::from_rgb888)
}

impl Describe for Video {
    const KIND: &'static str = "video";
    const DOC: &'static str = "The rectangle a video plane is shown in.";
    const GROUP: Group = Group::Media;

    const PROPERTIES: &'static [Property] = &[Property::new(
        "ground",
        PropertyKind::Color,
        "The letterbox colour behind the plane, as `\"#RRGGBB\"`.",
    )];

    fn get(&self, name: &str) -> Option<Value> {
        Some(match name {
            // Alpha is dropped rather than written: the ground is painted under
            // a hardware plane, where translucency has nothing to mean.
            "ground" => Value::Text(format!("#{:06X}", self.ground.to_argb8888() & 0x00FF_FFFF)),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            "ground" => {
                self.ground = ground_from_hex(&value.as_text()?).ok_or(Mismatch::WrongType {
                    expected: PropertyKind::Color,
                })?;
            }
            _ => return Err(Mismatch::Unknown),
        }
        Ok(())
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
