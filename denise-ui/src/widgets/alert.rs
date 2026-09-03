//! A coloured banner with a message.

use alloc::string::{String, ToString};

use denise::Pen;
use denise::{Point, Radius, Role};
use denise_text::{TextEngine, TextStyle};

use crate::widget::{MeasureCtx, Measured, Offer, PaintCtx, Widget};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Property, PropertyKind, ROLES, Value,
};
use crate::widgets::style::interactive_pair;

/// An inline banner: `Info`, `Success`, `Warning` or `Error`, with a message.
///
/// Not interactive, not focusable, not a tab stop. It sits *in* the layout, in
/// the place the thing it is about would be.
///
/// ```
/// # use denise_ui::Alert;
/// # use denise::theme::Role;
/// Alert::new(Role::Success, "Lagret").with_icon('✓');
/// Alert::new(Role::Error, "Kunne ikke lagre: disken er full");
/// ```
///
/// # This is a banner, not a dialog
///
/// Worth being explicit, because the word covers both. A *dialog* — something
/// that takes over, dims what is behind it and demands an answer — is
/// [`Ui::push_scene`](crate::Ui::push_scene), which already exists and already
/// dims and captures input. This is the strip of colour that reports something
/// happened.
///
/// Neither one opens a window. Denise is a single [`Surface`](denise::Surface):
/// on `denise-drm` there is no window system to open one in, and `denise-win32`,
/// `denise-macos` and `denise-activex` are *embedded* — the host owns the window
/// and Denise owns one rectangle inside it, so a control that spawned a
/// top-level window would escape its host's modality and outlive the dialog that
/// owns it.
///
/// An application that wants a native message box on a desktop build should call
/// the platform for one. It knows which build it is; the toolkit would have to
/// guess. That is the same conclusion the backend choice reached.
///
/// # Sizing
///
/// [`preferred_height`](Alert::preferred_height) reports what the message needs
/// once wrapped to a width — the query convention every sizable widget here
/// uses, called by the application and never by the tree.
#[derive(Clone, Debug)]
pub struct Alert {
    text: String,
    icon: Option<char>,
    role: Role,
    style: TextStyle,
}

impl Alert {
    /// A banner in `role` carrying `text`.
    ///
    /// `text` may contain `\n`, and is wrapped to the width it is given.
    pub fn new(role: Role, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            icon: None,
            role,
            style: TextStyle::built_in(16),
        }
    }

    /// Puts a character in front of the message.
    ///
    /// A `char`, not an icon set. There is no icon story in this toolkit and
    /// inventing one inside a banner would be the wrong place to start; whether
    /// `✓` or `⚠` actually draws depends on the font in use, and the built-in
    /// bitmap font has Latin and `æøå` and nothing else. `!` and `i` always work.
    pub fn with_icon(mut self, icon: char) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Sets the message's font and size.
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// The current message.
    #[inline]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Replaces the message.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    /// Replaces the message, reporting whether it actually changed.
    pub fn update(&mut self, text: &str) -> bool {
        let changed = self.text != text;
        if changed {
            self.text = text.to_string();
        }
        changed
    }

    /// Replaces the role.
    pub fn set_role(&mut self, role: Role) {
        self.role = role;
    }

    /// The current role.
    #[inline]
    pub const fn role(&self) -> Role {
        self.role
    }

    /// Replaces the leading character, or removes it.
    pub fn set_icon(&mut self, icon: Option<char>) {
        self.icon = icon;
    }

    /// Height this banner needs for its message wrapped to `width`.
    pub fn preferred_height(&self, engine: &mut TextEngine, width: i32) -> i32 {
        let inset = padding(self.style.size_px);
        // Computed first: `wrapped_height` takes the engine mutably too.
        let available = self.text_width(engine, width);
        engine.wrapped_height(self.style, &self.text, available) + inset * 2
    }

    /// Space the message has after the padding and any icon.
    fn text_width(&self, engine: &mut TextEngine, width: i32) -> i32 {
        let inset = padding(self.style.size_px);
        let icon = self.icon_width(engine);
        (width - inset * 2 - icon).max(1)
    }

    /// Width the icon and its gap occupy, or zero when there is none.
    fn icon_width(&self, engine: &mut TextEngine) -> i32 {
        let Some(icon) = self.icon else {
            return 0;
        };
        let mut buffer = [0u8; 4];
        let glyph = icon.encode_utf8(&mut buffer);
        engine.measure_line(self.style, glyph) + padding(self.style.size_px)
    }
}

/// Space between the message and the edge, on one side.
#[inline]
const fn padding(size_px: u16) -> i32 {
    let half = size_px as i32 / 2;
    if half < 4 { 4 } else { half }
}

impl<M: 'static> Widget<M> for Alert {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn measure(&self, ctx: &mut MeasureCtx<'_>, offered: Offer) -> Measured {
        // Height for a width, and no answer without one: wrapped text has no
        // height until it knows what it wraps to. A banner is as wide as you
        // make it, so there is no width to offer back.
        Measured {
            width: None,
            height: offered.width.map(|w| self.preferred_height(ctx.text, w)),
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        let bounds = ctx.bounds;
        if bounds.is_empty() {
            return;
        }
        // Both colours from one pairing, which is the whole reason this is a
        // widget rather than a `Panel` and a `Label`: a caller assembling it by
        // hand reaches for `BaseContent` and gets warning-coloured text nobody
        // can read on a warning-coloured background.
        let (fill, content) = interactive_pair(ctx.theme, self.role, ctx.state);
        canvas.fill_rounded_rect(bounds, ctx.theme.radius(Radius::Box), fill);

        let inset = padding(self.style.size_px);
        let line_height = ctx.text.line_height(self.style);
        let mut x = bounds.x + inset;

        if let Some(icon) = self.icon {
            let mut buffer = [0u8; 4];
            let glyph = icon.encode_utf8(&mut buffer);
            let width = ctx.text.measure_line(self.style, glyph);
            ctx.text.draw(
                canvas,
                self.style,
                Point::new(x, bounds.y + inset),
                glyph,
                content,
            );
            x += width + inset;
        }

        let available = (bounds.right() - inset - x).max(1);
        // Collected because `wrap` borrows the engine and drawing needs it again.
        // The lines borrow `self.text`, so only the slice headers are copied.
        let lines: alloc::vec::Vec<&str> = ctx.text.wrap(self.style, &self.text, available);
        for (index, line) in lines.iter().enumerate() {
            let y = bounds.y + inset + index as i32 * line_height;
            if y >= bounds.bottom() {
                // More message than banner. Clipped rather than drawn over
                // whatever is below, which the tree would do for us anyway —
                // stopping here just saves the glyph work.
                break;
            }
            ctx.text
                .draw(canvas, self.style, Point::new(x, y), line, content);
        }
    }
}

impl Describe for Alert {
    const KIND: &'static str = "alert";
    const DOC: &'static str =
        "A coloured banner saying something happened, in the place it happened.";
    const GROUP: Group = Group::Display;
    const ICON: &'static denise::icon::Icon = &super::icons::ALERT;

    const PROPERTIES: &'static [Property] = &[
        Property::new("text", PropertyKind::Text, "The message."),
        Property::new(
            "role",
            PropertyKind::Enum(ROLES),
            "The status this banner reports; an alert with no status is a label.",
        ),
        Property::new(
            "icon",
            PropertyKind::Text,
            "A single character drawn before the text.",
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
            // A banner without an icon has nothing to report, which is what
            // keeps `icon` out of a file that never set one.
            "icon" => Value::Text(self.icon?.to_string()),
            "size" => Value::Int(i32::from(self.style.size_px)),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            "text" => self.text = value.as_text()?,
            "role" => self.role = value.as_role()?,
            // The field holds one character, so a longer string keeps its
            // first: the property is described as a single character and a
            // banner is not the place to reject a form over a stray one. An
            // empty string removes the icon, which is the only way a file has
            // of saying so.
            "icon" => self.icon = value.as_text()?.chars().next(),
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

    use crate::widget::VisualState;

    fn engine() -> TextEngine {
        TextEngine::new()
    }

    /// The message wraps, so a long one is taller than a short one at the same
    /// width. Without this the banner is a one-liner with a scrollbar it does
    /// not have.
    #[test]
    fn a_longer_message_needs_a_taller_banner_at_the_same_width() {
        let mut engine = engine();
        let short = Alert::new(Role::Info, "Lagret").preferred_height(&mut engine, 200);
        let long = Alert::new(
            Role::Error,
            "Kunne ikke lagre fordi disken er full og det er ingen plass igjen",
        )
        .preferred_height(&mut engine, 200);
        assert!(long > short, "{long} is not taller than {short}");
    }

    /// And a wider banner needs fewer lines for the same message.
    #[test]
    fn a_wider_banner_needs_less_height_for_the_same_message() {
        let mut engine = engine();
        let alert = Alert::new(Role::Warning, "en to tre fire fem seks sju atte ni ti");
        let narrow = alert.preferred_height(&mut engine, 120);
        let wide = alert.preferred_height(&mut engine, 600);
        assert!(narrow > wide, "narrow {narrow} should exceed wide {wide}");
    }

    /// An icon takes space from the message, so the same text in the same width
    /// needs at least as much height with one as without.
    #[test]
    fn an_icon_takes_its_space_from_the_message() {
        let mut engine = engine();
        let text = "en to tre fire fem seks sju atte";
        let bare = Alert::new(Role::Info, text).preferred_height(&mut engine, 160);
        let iconed = Alert::new(Role::Info, text)
            .with_icon('!')
            .preferred_height(&mut engine, 160);
        assert!(
            iconed >= bare,
            "an icon should not make the banner shorter: {iconed} < {bare}"
        );
        assert!(Alert::new(Role::Info, text).icon_width(&mut engine) == 0);
        assert!(
            Alert::new(Role::Info, text)
                .with_icon('!')
                .icon_width(&mut engine)
                > 0
        );
    }

    /// A width too small to hold anything must still leave the message a column
    /// to wrap into, rather than a zero or negative one.
    #[test]
    fn an_absurdly_narrow_banner_still_leaves_a_column_for_the_text() {
        let mut engine = engine();
        for width in [-100, 0, 1, 5, 20] {
            let alert = Alert::new(Role::Error, "feil").with_icon('!');
            assert!(
                alert.text_width(&mut engine, width) >= 1,
                "width {width} left no room at all"
            );
            assert!(alert.preferred_height(&mut engine, width) > 0);
        }
    }

    /// An empty message is still a banner with a line's worth of height, not a
    /// zero-height sliver.
    #[test]
    fn an_empty_message_still_has_height() {
        let mut engine = engine();
        let height = Alert::new(Role::Info, "").preferred_height(&mut engine, 200);
        assert!(height > 0);
    }

    /// A message written every cycle should repaint only when it changes.
    #[test]
    fn writing_the_same_message_reports_no_change() {
        let mut alert = Alert::new(Role::Info, "Lagret");
        assert!(!alert.update("Lagret"));
        assert!(alert.update("Lagret kl. 12:01"));
    }

    /// The whole reason this is a widget rather than a `Panel` plus a `Label`:
    /// every role's text has to stay readable on its own background, in every
    /// theme. A caller assembling it by hand reaches for `BaseContent`.
    #[test]
    fn every_role_keeps_its_message_readable_in_every_theme() {
        use denise::theme::{AA_LARGE, contrast_x100};

        for theme in Theme::BUILT_IN {
            for role in [Role::Info, Role::Success, Role::Warning, Role::Error] {
                for state in [VisualState::NONE, VisualState::DISABLED] {
                    let (fill, content) = interactive_pair(&theme, role, state);
                    let ratio = contrast_x100(fill, content);
                    assert!(
                        ratio >= AA_LARGE,
                        "{} {role:?} {state:?}: message on banner is {ratio}, floor \
                         is {AA_LARGE}",
                        theme.name
                    );
                }
            }
        }
    }

    /// A multi-byte icon must not be sliced when it is encoded for measurement.
    #[test]
    fn a_multi_byte_icon_survives_being_measured() {
        let mut engine = engine();
        for icon in ['!', 'æ', '✓', '⚠'] {
            let alert = Alert::new(Role::Info, "melding").with_icon(icon);
            assert!(
                alert.icon_width(&mut engine) > 0,
                "{icon} measured as nothing"
            );
        }
    }
}
