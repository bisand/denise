//! Shared visual vocabulary, so the widgets agree with each other.

use denise::theme::{AA_LARGE, derive_content};
use denise::{Color, Point, Rect, Role, Size, Theme};
use denise_render::Canvas;
use denise_render::font::{self, BitmapFont};

use crate::widget::VisualState;

/// How much a hovered surface shifts towards its own content colour.
const HOVER_MIX: u8 = 24;
/// How much a pressed surface shifts. Larger, so press is unmistakably a
/// different state and not just a brighter hover.
const PRESS_MIX: u8 = 56;

/// Where text sits along one axis of its box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Align {
    /// Left, or top.
    #[default]
    Start,
    /// Centred.
    Center,
    /// Right, or bottom.
    End,
}

impl Align {
    /// Offset of a `content`-long run inside an `available`-long box.
    #[inline]
    pub const fn offset(self, available: i32, content: i32) -> i32 {
        match self {
            Align::Start => 0,
            Align::Center => (available - content) / 2,
            Align::End => available - content,
        }
    }
}

/// Surface and content colours for an interactive widget in a given state.
///
/// The shift is always *towards the widget's own content colour*, never towards
/// black or white. That is what keeps a hover readable on a light theme and on a
/// dark one without either being special-cased: the pair already guarantees
/// contrast, so moving along the line between them cannot break it.
pub(crate) fn interactive_pair(theme: &Theme, role: Role, state: VisualState) -> (Color, Color) {
    let (background, content) = theme.pair(role);
    if state.contains(VisualState::DISABLED) {
        // Disabled is a *recessed* surface with *derived* content, not a faded
        // one. Fading text towards its background is what produces the grey-on-
        // grey that nobody can read in daylight; deriving stops the moment it
        // clears 3:1, so it looks muted without becoming a guess.
        let background = theme.color(Role::Base200);
        return (background, derive_content(background, AA_LARGE));
    }
    if state.contains(VisualState::PRESSED) {
        return (background.mix(content, PRESS_MIX), content);
    }
    if state.contains(VisualState::HOVERED) {
        return (background.mix(content, HOVER_MIX), content);
    }
    (background, content)
}

/// Draws the keyboard focus ring, just inside `bounds`.
///
/// A ring rather than a colour change, because a panel driven only by Tab has to
/// show focus on a widget that may already be hovered or pressed.
pub(crate) fn focus_ring(theme: &Theme, bounds: Rect, radius: i32, canvas: &mut Canvas<'_>) {
    canvas.stroke_rounded_rect(
        bounds.inflate(-1),
        (radius - 1).max(0),
        2,
        theme.color(Role::Accent),
    );
}

/// Draws `text` inside `bounds` with the given alignment, and returns its extent.
pub(crate) fn draw_aligned(
    canvas: &mut Canvas<'_>,
    bounds: Rect,
    horizontal: Align,
    vertical: Align,
    scale: i32,
    text: &str,
    color: Color,
) -> Size {
    let font: &BitmapFont = &font::BUILT_IN;
    let extent = font.measure(text, scale);
    let at = Point::new(
        bounds.x + horizontal.offset(bounds.width, extent.width as i32),
        bounds.y + vertical.offset(bounds.height, extent.height as i32),
    );
    canvas.draw_text(font, at, scale, text, color);
    extent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_offsets() {
        assert_eq!(Align::Start.offset(100, 20), 0);
        assert_eq!(Align::Center.offset(100, 20), 40);
        assert_eq!(Align::End.offset(100, 20), 80);
        // Content wider than its box overflows to the left of it, not off the
        // right, which keeps the first characters readable.
        assert_eq!(Align::Center.offset(20, 100), -40);
    }

    #[test]
    fn every_state_keeps_the_pair_readable() {
        for theme in Theme::BUILT_IN {
            for role in [Role::Primary, Role::Secondary, Role::Accent, Role::Error] {
                for state in [
                    VisualState::NONE,
                    VisualState::HOVERED,
                    VisualState::PRESSED,
                    VisualState::DISABLED,
                ] {
                    let (background, content) = interactive_pair(&theme, role, state);
                    let ratio = denise::theme::contrast_x100(background, content);
                    assert!(
                        ratio >= AA_LARGE,
                        "{} {role:?} {state:?} is {ratio} against a floor of {AA_LARGE}",
                        theme.name
                    );
                }
            }
        }
    }
}
