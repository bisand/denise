//! Shared visual vocabulary, so the widgets agree with each other.

use denise::Pen;
use denise::theme::{AA_LARGE, contrast_x100, derive_content};
use denise::{Color, Point, Rect, Role, Size, Theme};
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
pub(crate) fn focus_ring(theme: &Theme, bounds: Rect, radius: i32, canvas: &mut Pen<'_>) {
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
    canvas: &mut Pen<'_>,
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

/// How a row is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowKind {
    /// Neither selected nor under the pointer.
    Resting,
    /// Under the pointer.
    Hovered,
    /// The selected row.
    Selected,
}

/// Fill and text colour for one row.
///
/// One function so the paint path and the contrast test cannot disagree about
/// what is actually drawn — and shared between [`List`](super::List) and
/// [`Table`](super::Table), so the disabled-selection answer lives once.
pub(crate) fn row_colors(
    theme: &Theme,
    state: VisualState,
    role: Role,
    kind: RowKind,
    enabled: bool,
) -> (Color, Color) {
    // The tree's HOVERED and PRESSED bits describe the *list*, not a row. Passing
    // them through would tint all twenty rows the moment the pointer entered the
    // widget, which is the opposite of what a hover highlight is for; the row
    // says it is hovered by being drawn in `Base200`.
    let state = state
        .set(VisualState::HOVERED, false)
        .set(VisualState::PRESSED, false);
    // Every pairing comes out of `interactive_pair`, so both colours of a row are
    // guaranteed against each other. A role is only ever guaranteed against its
    // own content — never against whatever surface it happens to sit on.
    let (surface, content) = match kind {
        // A disabled list still has to show which row is selected.
        // `interactive_pair` recesses *every* role to `Base200` when disabled, so
        // the selected row and a resting one would be the same drawing — the
        // mistake `RadioGroup` avoided by keeping a mark inside its disabled disc.
        // `Base300` is the theme's own next step up from that surface, and it
        // comes with its own content colour.
        RowKind::Selected if state.contains(VisualState::DISABLED) => theme.pair(Role::Base300),
        RowKind::Selected => interactive_pair(theme, role, state),
        RowKind::Hovered => interactive_pair(theme, Role::Base200, state),
        RowKind::Resting => interactive_pair(theme, Role::Base100, state),
    };
    if enabled {
        (surface, content)
    } else {
        // A row nobody can choose is de-emphasised — as far as this particular
        // pair can afford, which for a disabled list, or for a selected row in a
        // saturated role, is not at all. `muted` is what decides that.
        (surface, muted(surface, content))
    }
}

/// The row to highlight under the pointer.
///
/// `None` unless the tree still says the pointer is over this widget. The tree
/// clears `HOVERED` when the pointer moves to another widget and **does not send
/// this widget an event when it does** — [`InputEvent::PointerLeft`] never
/// reaches a widget at all. Trusting the remembered row on its own leaves a row
/// lit up under a pointer that is somewhere else entirely.
pub(crate) fn hovered_row(state: VisualState, remembered: Option<usize>) -> Option<usize> {
    if state.contains(VisualState::HOVERED) {
        remembered
    } else {
        None
    }
}

/// How long after a click a second one on the same row still counts as a pair.
///
/// The platform default nearly everywhere. Measured against
/// [`Ui::tick`](crate::Ui::tick)'s clock — an application that never calls
/// `tick` has a clock frozen at zero, and every second click reads as a pair.
pub(crate) const DOUBLE_CLICK_MS: u64 = 400;

/// What a click on a row turned out to mean.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Intent {
    Select,
    Activate,
}

/// Double-click detection, shared by [`List`](super::List) and
/// [`Table`](super::Table) so the pairing rules cannot drift apart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClickPair {
    last: Option<(usize, u64)>,
}

impl ClickPair {
    /// What a click on `row` at `at_ms` means, remembering it for the next one.
    ///
    /// With `single_click`, every click activates — the touch-panel answer,
    /// where a double-tap is unreliable and unexpected.
    pub(crate) fn classify(&mut self, row: usize, at_ms: u64, single_click: bool) -> Intent {
        if single_click {
            return Intent::Activate;
        }
        let pair = self
            .last
            .is_some_and(|(r, at)| r == row && at_ms >= at && at_ms - at <= DOUBLE_CLICK_MS);
        if pair {
            // Forgotten rather than updated, so a third click starts a new pair
            // instead of firing again — a triple-click is one activation.
            self.last = None;
            Intent::Activate
        } else {
            self.last = Some((row, at_ms));
            Intent::Select
        }
    }

    /// Drops a half-finished pair — called when the rows change, because a
    /// remembered click points at a row that may now be something else.
    pub(crate) fn forget(&mut self) {
        self.last = None;
    }
}

/// The leading, label and trailing boxes inside one row.
///
/// Shared by [`List`](super::List) and [`Tree`](super::Tree), so a row of one
/// and a row of the other put their columns in the same places.
///
/// The label takes what the other two leave. A row too narrow to hold all three
/// gives it nothing rather than a negative width — each column is clipped to
/// itself when it is drawn, so the result is text cut short rather than a label
/// running across the value at the other end of the row.
pub(crate) fn columns(row: Rect, pad: i32, leading: i32, trailing: i32) -> (Rect, Rect, Rect) {
    let left = row.x + pad;
    let right = (row.right() - pad).max(left);
    let box_of =
        |start: i32, end: i32| Rect::from_edges(start, row.y, end.max(start), row.bottom());

    let leading_box = box_of(left, (left + leading).min(right));
    let trailing_box = box_of((right - trailing).max(left), right);
    // Clamped into the row's own span, so a column that was pushed outside by a
    // rectangle too narrow for it does not drag the label out with it.
    let start = if leading > 0 {
        (leading_box.right() + pad).clamp(left, right)
    } else {
        left
    };
    let end = if trailing > 0 {
        (trailing_box.x - pad).clamp(left, right)
    } else {
        right
    };
    (leading_box, box_of(start, end), trailing_box)
}
