//! A short string in a coloured pill.

use alloc::string::{String, ToString};

use denise::Role;
use denise_render::Canvas;
use denise_text::{TextEngine, TextStyle};

use crate::widget::{PaintCtx, Widget};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Property, PropertyKind, ROLES, Value,
};
use crate::widgets::style::{Align, draw_aligned, interactive_pair};

/// A count, a status, a short word, in a stadium of its role's colour.
///
/// Not interactive, not focusable, not a tab stop. It is a [`Label`] that
/// carries a colour and a shape.
///
/// ```
/// # use denise_ui::widgets::Badge;
/// # use denise::theme::Role;
/// Badge::new("3");
/// Badge::new("PÅ").with_role(Role::Success);
/// Badge::new("FEIL").with_role(Role::Error);
/// ```
///
/// # Sizing, and why this is not the start of a layout engine
///
/// A badge wants to be as wide as its text, which raises the question of who
/// decides. The answer here is the one four widgets already use:
/// [`preferred_width`](Badge::preferred_width) and
/// [`preferred_height`](Badge::preferred_height) are **queries the application
/// makes**, and the tree never calls them.
///
/// That is the whole distinction. An intrinsic-size *protocol* is one where the
/// tree asks each widget how big it wants to be and then places it — which is a
/// layout engine, and a different toolkit. Here the application asks, does its
/// own arithmetic, and passes a rectangle, exactly as it does for every other
/// node. [`Button`](super::Button), [`Checkbox`](super::Checkbox),
/// [`Toggle`](super::Toggle) and [`RadioGroup`](super::RadioGroup) all work this
/// way already.
///
/// Given a rectangle that is not the preferred one, a badge fills it and centres
/// its text. Text too long is clipped to the badge rather than spilling out of
/// it — the tree clips the canvas to the widget's bounds, so that comes free.
///
/// [`Label`]: super::Label
#[derive(Clone, Debug)]
pub struct Badge {
    text: String,
    role: Role,
    style: TextStyle,
}

impl Badge {
    /// A badge in [`Role::Primary`], in the built-in font at 14 px.
    ///
    /// A size smaller than the surrounding text on purpose: a badge is an
    /// annotation on something else, and one set at the body size reads as a
    /// button.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            role: Role::Primary,
            style: TextStyle::built_in(14),
        }
    }

    /// Sets the colour role. The text colour comes from the theme's pairing, so
    /// it stays readable whichever role and theme are chosen.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
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

    /// The current text.
    #[inline]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Replaces the text.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    /// Replaces the text, reporting whether it actually changed.
    ///
    /// The [`Label::update`](super::Label::update) pattern. A badge is usually a
    /// count written every cycle, and repainting for a number that did not move
    /// is how an idle panel stops being idle.
    pub fn update(&mut self, text: &str) -> bool {
        let changed = self.text != text;
        if changed {
            self.text = text.to_string();
        }
        changed
    }

    /// Replaces the colour role.
    pub fn set_role(&mut self, role: Role) {
        self.role = role;
    }

    /// Replaces the font and size.
    pub fn set_style(&mut self, style: TextStyle) {
        self.style = style;
    }

    /// The font and size the text draws in.
    #[inline]
    pub const fn style(&self) -> TextStyle {
        self.style
    }

    /// Width this badge wants: its text plus padding, never less than its height.
    ///
    /// The floor is what makes a one-character badge a circle rather than a
    /// squashed pill — a `3` in a stadium 12 wide and 20 tall looks like a
    /// mistake, and a count of one digit is the commonest badge there is.
    pub fn preferred_width(&self, engine: &mut TextEngine) -> i32 {
        let text = engine.measure_line(self.style, &self.text);
        (text + padding(self.style.size_px) * 2).max(self.preferred_height(engine))
    }

    /// Height this badge wants: its text plus padding.
    pub fn preferred_height(&self, engine: &mut TextEngine) -> i32 {
        // Measured rather than taken from the nominal size, because a font's line
        // height is its own business — the built-in bitmap font and a TrueType
        // face at the same `size_px` do not agree.
        let extent = engine.measure(self.style, &self.text);
        extent.height as i32 + padding(self.style.size_px)
    }
}

/// Space between the text and the edge, on one side.
#[inline]
const fn padding(size_px: u16) -> i32 {
    // Half the nominal size: 7px each side at 14px, which is the proportion a
    // pill needs before it stops looking like a rectangle with round corners.
    let half = size_px as i32 / 2;
    if half < 2 { 2 } else { half }
}

impl<M: 'static> Widget<M> for Badge {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        let bounds = ctx.bounds;
        if bounds.is_empty() {
            return;
        }
        // A stadium: the radius is half the height, capped by half the width so a
        // badge narrower than it is tall does not ask for a radius wider than the
        // rectangle it is rounding.
        let radius = (bounds.height / 2).min(bounds.width / 2);

        let (fill, content) = interactive_pair(ctx.theme, self.role, ctx.state);
        canvas.fill_rounded_rect(bounds, radius, fill);

        if self.text.is_empty() {
            return;
        }
        // Centred, and clipped by the tree to these bounds — text longer than the
        // badge is cut off at the pill rather than running across whatever is
        // beside it.
        draw_aligned(
            canvas,
            ctx.text,
            self.style,
            bounds,
            (Align::Center, Align::Center),
            &self.text,
            content,
        );
    }
}

impl Describe for Badge {
    const KIND: &'static str = "badge";
    const DOC: &'static str = "A count, a status or a short word, in a pill of its role's colour.";
    const GROUP: Group = Group::Display;

    const PROPERTIES: &'static [Property] = &[
        Property::new("text", PropertyKind::Text, "The text."),
        Property::new(
            "role",
            PropertyKind::Enum(ROLES),
            "Colour role the pill is filled with; the text takes its content colour.",
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
            "size" => Value::Int(i32::from(self.style.size_px)),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            "text" => self.text = value.as_text()?,
            "role" => self.role = value.as_role()?,
            "size" => self.style.size_px = value.as_size()?,
            _ => return Err(Mismatch::Unknown),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use denise::Theme;
    use denise::theme;

    use crate::widget::VisualState;

    /// A one-character badge is a circle, not a squashed pill. The commonest
    /// badge there is, and the one a text-width rule alone gets wrong.
    #[test]
    fn a_single_character_badge_is_at_least_as_wide_as_it_is_tall() {
        let mut engine = TextEngine::new();
        let badge = Badge::new("3");
        let width = badge.preferred_width(&mut engine);
        let height = badge.preferred_height(&mut engine);
        assert!(
            width >= height,
            "a single digit gave {width}x{height}, which is a squashed pill"
        );
    }

    /// And a long one grows with its text rather than staying square.
    #[test]
    fn a_longer_badge_grows_with_its_text() {
        let mut engine = TextEngine::new();
        let short = Badge::new("1").preferred_width(&mut engine);
        let medium = Badge::new("PÅ").preferred_width(&mut engine);
        let long = Badge::new("VEDLIKEHOLD").preferred_width(&mut engine);
        assert!(medium >= short);
        assert!(long > medium, "{long} is not wider than {medium}");
    }

    /// The width is the measured text plus padding, not a character count times
    /// anything — which is the only thing that works with a proportional font.
    #[test]
    fn the_width_is_the_measured_text_plus_padding() {
        let mut engine = TextEngine::new();
        let badge = Badge::new("VEDLIKEHOLD");
        let text = engine.measure_line(badge.style(), badge.text());
        assert_eq!(
            badge.preferred_width(&mut engine),
            text + padding(14) * 2,
            "padding should be one half-size at each end"
        );
    }

    /// An empty badge is still a badge — a dot, not a zero-width nothing.
    #[test]
    fn an_empty_badge_still_has_a_size() {
        let mut engine = TextEngine::new();
        let badge = Badge::new("");
        assert!(badge.preferred_width(&mut engine) > 0);
        assert!(badge.preferred_height(&mut engine) > 0);
    }

    /// A count written every cycle should repaint only when the number moves.
    #[test]
    fn writing_the_same_text_reports_no_change() {
        let mut badge = Badge::new("3");
        assert!(!badge.update("3"));
        assert!(badge.update("4"));
        assert_eq!(badge.text(), "4");
    }

    /// The text is drawn in the role's own content colour, so every role stays
    /// readable in every theme. A badge is small text on a saturated fill, which
    /// is exactly where a hard-coded white would fail.
    #[test]
    fn every_role_keeps_its_text_readable_in_every_theme() {
        use denise::theme::{AA_LARGE, contrast_x100};

        for theme in Theme::BUILT_IN {
            for role in [
                Role::Primary,
                Role::Secondary,
                Role::Accent,
                Role::Neutral,
                Role::Info,
                Role::Success,
                Role::Warning,
                Role::Error,
            ] {
                for state in [VisualState::NONE, VisualState::DISABLED] {
                    let (fill, content) = interactive_pair(&theme, role, state);
                    let ratio = contrast_x100(fill, content);
                    assert!(
                        ratio >= AA_LARGE,
                        "{} {role:?} {state:?}: text on badge is {ratio}, floor is \
                         {AA_LARGE}",
                        theme.name
                    );
                }
            }
        }
    }

    /// Padding never collapses, however small the font.
    #[test]
    fn padding_survives_an_absurdly_small_font() {
        assert!(padding(0) >= 2);
        assert!(padding(1) >= 2);
        assert!(padding(u16::MAX) > 0);
    }

    /// Keeps the theme import honest: the role colours come from a theme, never
    /// from a constant in this file.
    #[test]
    fn the_default_role_is_a_theme_role_not_a_colour() {
        let badge = Badge::new("3");
        assert_eq!(badge.role, Role::Primary);
        let _ = theme::DARK.color(badge.role);
    }
}
