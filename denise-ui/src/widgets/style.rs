//! Shared visual vocabulary, so the widgets agree with each other.

use denise::theme::{AA_LARGE, contrast_x100, derive_content};
use denise::{Color, Point, Rect, Role, Size, Theme};
use denise_render::Canvas;
use denise_text::{TextEngine, TextStyle};

use crate::widget::VisualState;

/// How much a hovered surface shifts towards its own content colour.
const HOVER_MIX: u8 = 24;
/// How much a pressed surface shifts. Larger, so press is unmistakably a
/// different state and not just a stronger hover.
///
/// This cannot be turned up freely: moving a background towards its own text
/// colour costs contrast, and the light theme's `primary` pair breaks 3:1 at 72.
/// `every_state_keeps_the_pair_readable` is what found that, and is what will
/// find it again if a future theme has a tighter pair than today's do.
const PRESS_MIX: u8 = 64;

/// How far a de-emphasised label is moved towards the surface behind it.
///
/// For text that is *present but not the point*: an unselected tab, a row a list
/// will not let you choose. Enough to make the emphasised one obviously
/// emphasised, and not so far that the others stop being readable.
///
/// Swept against the built-in themes: 96 leaves the light theme at 2.93:1, under
/// the 3:1 floor, and 64 leaves `Base100` at 3.84:1 in the worst of the three.
/// The same number `PRESS_MIX` arrived at, for the same reason.
///
/// Not enough on its own, though — see [`muted`].
const MUTE: u8 = 64;

/// `content` moved towards `surface`, but only as far as it can afford to go.
///
/// De-emphasis costs contrast, and **not every pair has contrast to spend.** Two
/// separate widgets found that out the hard way, and this is the rule that covers
/// both:
///
/// - [`interactive_pair`] *derives* a disabled widget's content by mixing until it
///   **just** clears the floor. Muting that drops a label to 2.33:1.
/// - A theme's saturated pairs are only guaranteed to *reach* the floor. Muting
///   the dark theme's `Primary` content leaves 2.94:1, so a selected row that was
///   also disabled would have been unreadable in one theme out of three.
///
/// A pair with room to give — `Base100` against `BaseContent` is near-black on
/// near-white — mutes as asked. One that has none is returned unchanged, because
/// legible and undifferentiated beats differentiated and illegible.
pub(crate) fn muted(surface: Color, content: Color) -> Color {
    let muted = content.mix(surface, MUTE);
    if contrast_x100(surface, muted) >= AA_LARGE {
        muted
    } else {
        content
    }
}

/// Which way a widget runs.
///
/// Shared rather than owned by one widget: a divider, and later a slider or a
/// group of options, all mean the same thing by it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Orientation {
    /// Left to right, splitting a column of content.
    #[default]
    Horizontal,
    /// Top to bottom, splitting a row.
    Vertical,
}

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
///
/// Measurement goes through the engine, so the box a widget centres in is the box
/// the glyphs actually occupy — including with a proportional font, where the
/// answer is not the character count times anything.
pub(crate) fn draw_aligned(
    canvas: &mut Canvas<'_>,
    engine: &mut TextEngine,
    style: TextStyle,
    bounds: Rect,
    align: (Align, Align),
    text: &str,
    color: Color,
) -> Size {
    let extent = engine.measure(style, text);
    let at = Point::new(
        bounds.x + align.0.offset(bounds.width, extent.width as i32),
        bounds.y + align.1.offset(bounds.height, extent.height as i32),
    );
    engine.draw(canvas, style, at, text, color);
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
