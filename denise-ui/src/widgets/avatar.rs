//! A person, as a picture or as their initials.

use alloc::string::String;
use alloc::vec::Vec;

use denise::{Point, Rect, Role, Size};
use denise_render::Canvas;
use denise_text::TextStyle;

use crate::widget::{PaintCtx, Widget};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, PRESENCES, Property, PropertyKind, ROLES, Value,
};
use crate::widgets::image::{Fit, Image};
use crate::widgets::style::{Align, draw_aligned, interactive_pair};

/// The roles an avatar's disc is coloured from when nobody chose one.
///
/// The theme's own accents, so a derived colour is still a *theme* colour: it
/// survives a theme swap, and it comes with the contrast-checked content colour
/// the theme guarantees for it. Inventing an RGB triple here would do neither.
const PALETTE: [Role; 6] = [
    Role::Primary,
    Role::Secondary,
    Role::Accent,
    Role::Info,
    Role::Success,
    Role::Error,
];

/// How far in from the edge the presence dot sits, as a percentage of the
/// radius. At 71% the dot's centre is on the circle at 45°, where a round crop
/// and a square one agree; a little further in keeps it clear of both.
const DOT_INSET_PERCENT: i32 = 68;

/// The dot's radius, as a percentage of the avatar's.
const DOT_PERCENT: i32 = 26;

/// Whether an avatar shows a presence dot, and which one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Presence {
    /// A filled dot in [`Role::Success`].
    Online,
    /// A filled dot in [`Role::Base300`] — present but not active.
    Offline,
    /// A filled dot in [`Role::Warning`].
    Busy,
}

impl Presence {
    /// The role this dot is drawn in.
    const fn role(self) -> Role {
        match self {
            Presence::Online => Role::Success,
            Presence::Offline => Role::Base300,
            Presence::Busy => Role::Warning,
        }
    }
}

/// A person's picture, or their initials on a coloured disc.
///
/// ```
/// # use denise_ui::widgets::{Avatar, Presence};
/// # use denise::Size;
/// # let (pixels, size) = (vec![0u32; 64 * 64], Size::new(64, 64));
/// Avatar::new(pixels, size);                      // a photo, circular
/// Avatar::initials("Ola Nordmann");               // OL, on a derived colour
/// Avatar::initials("Kari").with_presence(Presence::Online);
/// ```
///
/// Not interactive and not focusable, like [`Label`](super::Label) and
/// [`Image`]: an avatar inside a button never swallows the click.
///
/// # The fallback is the point
///
/// Cropping a picture to a circle is one call —
/// [`Image`] with [`Fit::Cover`] and a full corner radius, which is what this
/// widget uses rather than reimplementing. What earns a widget is what happens
/// when there is **no** picture, which on a real panel is most of the time:
/// initials, centred, on a colour that is legible and that is *the same colour
/// every time for the same person*, so a list of operators stays recognisable
/// between sessions.
///
/// A picture whose buffer does not match the size it claims falls back to the
/// initials too. A broken asset on a kiosk should still say who it is.
///
/// # The colour is derived, not invented
///
/// From the initials, into the theme's own accent roles — so it swaps with the
/// theme and carries the contrast-checked content colour the theme guarantees.
/// [`with_role`](Avatar::with_role) overrides it where the caller knows better.
#[derive(Clone, Debug)]
pub struct Avatar {
    picture: Option<Image>,
    /// The name the initials were taken from, kept verbatim.
    ///
    /// `initials_of` is lossy and not idempotent — "Ada Lovelace" reduces to
    /// "AL", and "AL" reduces again to "A" — so a widget that kept only the
    /// reduction could not tell anybody what it was given. A property inspector
    /// reading this back would show "AL" where the author typed a name, and
    /// saving would write the reduction into the form file, losing a little more
    /// of it every round. So the name is kept and the initials are derived.
    name: String,
    initials: String,
    role: Option<Role>,
    /// `None` is a circle: half the shorter side, whatever that turns out to be.
    radius: Option<i32>,
    ring: Option<Role>,
    presence: Option<Presence>,
    style: TextStyle,
}

impl Avatar {
    /// An avatar showing a picture, cropped to the shape.
    ///
    /// `pixels` is premultiplied `0xAARRGGBB` — [`Image`]'s contract exactly.
    /// A buffer too small for `size` falls back to the initials, which are
    /// empty unless [`with_initials`](Avatar::with_initials) supplies them.
    pub fn new(pixels: Vec<u32>, size: Size) -> Self {
        Self {
            picture: Some(Image::new(pixels, size).with_fit(Fit::Cover)),
            name: String::new(),
            initials: String::new(),
            role: None,
            radius: None,
            ring: None,
            presence: None,
            style: TextStyle::built_in(16),
        }
    }

    /// An avatar showing initials taken from `name` — see [`initials_of`].
    pub fn initials(name: &str) -> Self {
        Self {
            picture: None,
            name: name.into(),
            initials: initials_of(name),
            role: None,
            radius: None,
            ring: None,
            presence: None,
            style: TextStyle::built_in(16),
        }
    }

    /// Sets the initials shown when there is no picture, or the picture is
    /// broken. Taken from `name` the same way [`Avatar::initials`] takes them.
    pub fn with_initials(mut self, name: &str) -> Self {
        self.set_name(name);
        self
    }

    /// The name the initials were taken from, as it was given.
    ///
    /// Empty for an avatar built from a picture alone. See
    /// [`initials_text`](Avatar::initials_text) for what is actually drawn.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Replaces the name, and the initials derived from it.
    pub fn set_name(&mut self, name: &str) {
        self.name.clear();
        self.name.push_str(name);
        self.initials = initials_of(name);
    }

    /// Overrides the colour derived from the initials.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = Some(role);
        self
    }

    /// Makes this a rounded square of `radius` instead of a circle.
    pub fn with_corner_radius(mut self, radius: i32) -> Self {
        self.radius = Some(radius.max(0));
        self
    }

    /// Draws a ring around the avatar, in `role`.
    pub fn with_ring(mut self, role: Role) -> Self {
        self.ring = Some(role);
        self
    }

    /// Puts a presence dot at the lower right.
    pub fn with_presence(mut self, presence: Presence) -> Self {
        self.presence = Some(presence);
        self
    }

    /// Sets the initials' font and size.
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// The initials this avatar falls back to.
    #[inline]
    pub fn initials_text(&self) -> &str {
        &self.initials
    }

    /// Replaces the picture. Reach this through
    /// [`Ui::widget_mut`](crate::Ui::widget_mut), which marks the node dirty.
    pub fn set_picture(&mut self, pixels: Vec<u32>, size: Size) {
        self.picture = Some(Image::new(pixels, size).with_fit(Fit::Cover));
    }

    /// Drops the picture, falling back to the initials.
    pub fn clear_picture(&mut self) {
        self.picture = None;
    }

    /// The role this avatar's disc is drawn in: the one set, or the one derived
    /// from the initials.
    pub fn disc_role(&self) -> Role {
        self.role.unwrap_or_else(|| role_for(&self.initials))
    }

    /// The square this avatar occupies inside `bounds`, and its corner radius.
    fn square(&self, bounds: Rect) -> (Rect, i32) {
        let side = bounds.width.min(bounds.height).max(0);
        let square = Rect::new(
            bounds.x + (bounds.width - side) / 2,
            bounds.y + (bounds.height - side) / 2,
            side,
            side,
        );
        // A circle is just the full radius, so the two shapes are one code path
        // and the avatar crop needs no special case in the rasteriser either.
        let radius = self.radius.unwrap_or(side / 2).min(side / 2);
        (square, radius)
    }
}

/// Up to two initials from a name.
///
/// The first letter of the first word and of the last, so "Ola Nordmann" gives
/// `ON` and "Kari" gives `K`. Words are split on whitespace and on the
/// punctuation a name list actually contains — commas from "Nordmann, Ola",
/// hyphens from double-barrelled surnames — and anything that is not a letter
/// or a digit is skipped, so an email address or a stray emoji does not become
/// somebody's initials.
///
/// Uppercased through `char::to_uppercase`, so `æ` becomes `Æ` and the built-in
/// font can draw it. A name with no letters at all gives an empty string, and
/// an avatar with no initials is a plain disc — which is still better than a
/// hole.
pub fn initials_of(name: &str) -> String {
    let mut words = name
        .split(|c: char| c.is_whitespace() || c == ',' || c == '-' || c == '_' || c == '.')
        .filter_map(|word| word.chars().find(|c| c.is_alphanumeric()));

    let mut out = String::new();
    let first = words.next();
    let last = words.next_back();
    for c in first.into_iter().chain(last) {
        out.extend(c.to_uppercase());
    }
    out
}

/// The presence dot's centre and radius for an avatar occupying `square`.
///
/// Deliberately unclamped. [`DOT_INSET_PERCENT`] plus [`DOT_PERCENT`] is 94, so
/// the dot's far edge lands at 1.94 of the avatar's radius against the 2 it
/// has — the constants are what keep it inside, not a guard at the point of
/// use. An earlier version clamped here as well, and a mutation removing the
/// clamp changed nothing at any size, which is how it came out: it was dead
/// code standing in for the test that should have been pinning the constants.
/// That test is now [`the_presence_dot_never_leaves_the_avatar`].
///
/// The dot does straddle the *circle*, which is intended — that is what a
/// presence dot looks like — but it never leaves the square, so the crop never
/// eats it.
///
/// `None` for an avatar too small to hold one. Below four pixels the floors
/// that keep a dot at least one pixel across make it as wide as the avatar and
/// centred on it, and a dot that cannot sit in a corner is better left out than
/// drawn over a face that is already barely legible.
fn dot_at(square: Rect) -> Option<(Point, i32)> {
    let r = (square.width / 2).max(1);
    let dot = (r * DOT_PERCENT / 100).max(1);
    let inset = r * DOT_INSET_PERCENT / 100;
    let centre = Point::new(square.x + r + inset, square.y + r + inset);
    let extent = Rect::new(centre.x - dot, centre.y - dot, dot * 2, dot * 2);
    // Two conditions, both mechanical rather than a guessed pixel threshold:
    // the dot has to fit, and it has to actually be *offset* — at three pixels
    // and under the inset floors to zero, which would put a dot the size of the
    // avatar exactly on top of it.
    (inset > 0 && square.contains_rect(&extent)).then_some((centre, dot))
}

/// Which palette role a set of initials lands on.
///
/// Deterministic and stable: the same initials give the same colour on every
/// run and every machine, which is the whole point — a face you recognise in a
/// list. A plain weighted sum is enough; this is picking one of six buckets,
/// not defending against anything.
fn role_for(initials: &str) -> Role {
    let sum = initials.chars().enumerate().fold(0u32, |acc, (i, c)| {
        acc.wrapping_add((c as u32).wrapping_mul(31 + i as u32))
    });
    PALETTE[(sum as usize) % PALETTE.len()]
}

impl<M: 'static> Widget<M> for Avatar {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        let (square, radius) = self.square(ctx.bounds);
        if square.is_empty() {
            return;
        }

        // A picture that cannot be drawn — a buffer shorter than the size it
        // claims — must fall back rather than leave a hole, so this asks the
        // image whether it *will* draw before deciding.
        let drew_picture = match &self.picture {
            Some(picture) if picture.is_drawable() => {
                picture.paint_at(square, radius, canvas);
                true
            }
            _ => false,
        };

        if !drew_picture {
            let (disc, content) = interactive_pair(ctx.theme, self.disc_role(), ctx.state);
            canvas.fill_rounded_rect(square, radius, disc);
            if !self.initials.is_empty() {
                draw_aligned(
                    canvas,
                    ctx.text,
                    self.style,
                    square,
                    (Align::Center, Align::Center),
                    &self.initials,
                    content,
                );
            }
        }

        if let Some(role) = self.ring {
            let (color, _) = interactive_pair(ctx.theme, role, ctx.state);
            canvas.stroke_rounded_rect(square, radius, ctx.theme.metrics.border.max(1), color);
        }

        if let Some(presence) = self.presence
            && let Some((centre, dot)) = dot_at(square)
        {
            // A rim in the surface behind it, so the dot reads as sitting on
            // the avatar rather than being part of the picture under it.
            let (color, _) = interactive_pair(ctx.theme, presence.role(), ctx.state);
            canvas.fill_circle(centre, dot, ctx.theme.color(Role::Base100));
            canvas.fill_circle(centre, (dot - ctx.theme.metrics.border).max(1), color);
        }
    }
}

impl Describe for Avatar {
    const KIND: &'static str = "avatar";
    const DOC: &'static str = "A person: their picture, or their initials on a coloured disc.";
    const GROUP: Group = Group::Media;

    const PROPERTIES: &'static [Property] = &[
        Property::new(
            "src",
            PropertyKind::Asset,
            "A picture, as a path relative to the form file. Without one, the initials are drawn.",
        ),
        Property::new(
            "initials",
            PropertyKind::Text,
            "A name; the widget takes its initials.",
        ),
        Property::new(
            "role",
            PropertyKind::Enum(ROLES),
            "The disc behind the initials — derived from the initials when unset, so a column of avatars is not one colour.",
        ),
        Property::new(
            "radius",
            PropertyKind::Int { min: 0, max: 256 },
            "Corner radius in pixels; `0` is a square, unset is a circle.",
        )
        .in_pixels(),
        Property::new("ring", PropertyKind::Enum(ROLES), "A ring around the disc."),
        Property::new("presence", PropertyKind::Enum(PRESENCES), "The status dot."),
        Property::new(
            "size",
            PropertyKind::Int { min: 6, max: 96 },
            "Initials' text size in logical pixels.",
        )
        .in_pixels(),
    ];

    fn get(&self, name: &str) -> Option<Value> {
        Some(match name {
            // The widget holds decoded pixels and never saw the path they came
            // from, so there is nothing to report. See the `describe` module.
            "src" => return None,
            // What comes back is the *initials*, not the name they were taken
            // from — the name is not kept. Setting it again is a no-op, since
            // `initials_of` leaves an already-reduced pair alone.
            "initials" if self.name.is_empty() => return None,
            "initials" => Value::text(self.name.as_str()),
            // Each of these is unset until somebody sets it, and the widget has
            // a real behaviour for unset — a derived colour, a circle, no ring,
            // no dot — that no value in the file would say better.
            "role" => Value::role(self.role?),
            "radius" => Value::Int(self.radius?),
            "ring" => Value::role(self.ring?),
            "presence" => Value::presence(self.presence?),
            "size" => Value::Int(i32::from(self.style.size_px)),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            "src" => return Err(Mismatch::Supplied),
            // Reduced on the way in, exactly as `with_initials` reduces it: the
            // field's invariant is that it already holds what gets drawn.
            "initials" => self.set_name(&value.as_text()?),
            "role" => self.role = Some(value.as_role()?),
            // Clamped as `with_corner_radius` clamps it; `square` takes care of
            // the upper bound, which depends on a size only paint knows.
            "radius" => self.radius = Some(value.as_int()?.max(0)),
            "ring" => self.ring = Some(value.as_role()?),
            "presence" => self.presence = Some(value.as_presence()?),
            "size" => self.style.size_px = value.as_size()?,
            _ => return Err(Mismatch::Unknown),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_come_from_the_first_and_last_word() {
        assert_eq!(initials_of("Ola Nordmann"), "ON");
        assert_eq!(initials_of("Kari"), "K");
        assert_eq!(initials_of("Ola Kristian Nordmann"), "ON");
        assert_eq!(initials_of("Nordmann, Ola"), "NO");
        assert_eq!(initials_of("anne-marie"), "AM");
    }

    #[test]
    fn initials_are_uppercased_including_the_norwegian_letters() {
        assert_eq!(initials_of("øystein åsen"), "ØÅ");
        assert_eq!(initials_of("ærlig"), "Æ");
    }

    /// A name field on a real panel contains addresses, numbers and worse.
    #[test]
    fn names_that_are_not_names_still_produce_something_or_nothing() {
        assert_eq!(initials_of(""), "");
        assert_eq!(initials_of("   "), "");
        assert_eq!(initials_of("!!! ???"), "", "no letters at all");
        assert_eq!(initials_of("ola@example.com"), "OC", "split on the dot");
        assert_eq!(initials_of("3M"), "3", "a digit is a usable initial");
        assert_eq!(initials_of("🙂 Ola"), "O", "the emoji is skipped");
    }

    /// The property the whole derivation exists for: a face you recognise.
    #[test]
    fn the_same_name_always_gets_the_same_colour() {
        for name in ["Ola Nordmann", "Kari", "øystein åsen", ""] {
            let once = Avatar::initials(name).disc_role();
            let again = Avatar::initials(name).disc_role();
            assert_eq!(once, again, "{name}");
        }
        // And the initials are what decide it, not the whole name — two people
        // written differently but abbreviating the same must match.
        assert_eq!(
            Avatar::initials("Ola Nordmann").disc_role(),
            Avatar::initials("Olav Nilsen").disc_role()
        );
    }

    /// A palette that mostly returns one colour is not a palette.
    #[test]
    fn different_names_spread_across_the_palette() {
        let names = [
            "Ola Nordmann",
            "Kari Traa",
            "Per Hansen",
            "Ida Berg",
            "Nils Aas",
            "Eva Lund",
            "Tor Dahl",
            "Siv Moen",
            "Jon Vik",
            "Ann Ruud",
            "Leif Rud",
            "Mia Sund",
        ];
        let mut seen = Vec::new();
        for name in names {
            let role = Avatar::initials(name).disc_role();
            if !seen.contains(&role) {
                seen.push(role);
            }
        }
        assert!(
            seen.len() >= 4,
            "twelve names landed on only {} of {} colours",
            seen.len(),
            PALETTE.len()
        );
    }

    /// An explicit role wins over the derived one.
    #[test]
    fn an_explicit_role_overrides_the_derivation() {
        let a = Avatar::initials("Ola Nordmann").with_role(Role::Neutral);
        assert_eq!(a.disc_role(), Role::Neutral);
    }

    /// The avatar is square and centred, whatever rectangle it is given — an
    /// avatar squashed into an ellipse is not an avatar.
    #[test]
    fn the_avatar_squares_itself_inside_any_rectangle() {
        for bounds in [
            Rect::new(0, 0, 100, 40),
            Rect::new(10, 20, 40, 100),
            Rect::new(-5, -5, 60, 60),
            Rect::new(0, 0, 1, 1),
            Rect::new(0, 0, 0, 50),
        ] {
            let (square, radius) = Avatar::initials("ON").square(bounds);
            assert_eq!(square.width, square.height, "{bounds:?} is not square");
            assert_eq!(
                square.width,
                bounds.width.min(bounds.height).max(0),
                "{bounds:?}"
            );
            assert!(bounds.contains_rect(&square), "{bounds:?}: escaped");
            assert!(radius <= square.width / 2, "{bounds:?}: radius too large");
        }
    }

    /// A circle is the full radius, and asking for more does not overshoot it.
    #[test]
    fn a_circle_is_the_full_radius_and_never_more() {
        let bounds = Rect::new(0, 0, 40, 40);
        let (_, circle) = Avatar::initials("ON").square(bounds);
        assert_eq!(circle, 20);
        let (_, asked) = Avatar::initials("ON")
            .with_corner_radius(9_999)
            .square(bounds);
        assert_eq!(asked, 20, "a huge radius is still just a circle");
        let (_, rounded) = Avatar::initials("ON").with_corner_radius(6).square(bounds);
        assert_eq!(rounded, 6);
    }

    /// The constants are the containment, so this is what holds them to it: a
    /// dot that leaves the square is a dot the round crop eats.
    #[test]
    fn the_presence_dot_never_leaves_the_avatar() {
        for side in 1..400 {
            let square = Rect::new(10, 20, side, side);
            let Some((centre, dot)) = dot_at(square) else {
                assert!(side < 4, "side {side} refused a dot it had room for");
                continue;
            };
            let box_of_dot = Rect::new(centre.x - dot, centre.y - dot, dot * 2, dot * 2);
            assert!(
                square.contains_rect(&box_of_dot),
                "side {side}: the dot {box_of_dot:?} left the avatar {square:?}"
            );
            // And it is actually at the lower right, not hiding in the middle.
            assert!(
                centre.x > square.x + side / 2,
                "side {side}: the dot is not at the lower right"
            );
        }
        assert!(
            dot_at(Rect::new(0, 0, 24, 24)).is_some(),
            "an ordinary size"
        );
    }

    /// The initials have to be readable on the disc they sit on, in every theme
    /// and state — the floor every widget here is held to.
    #[test]
    fn initials_are_readable_on_their_disc_in_every_theme() {
        use crate::widget::VisualState;
        use denise::Theme;
        use denise::theme::{AA_LARGE, contrast_x100};

        for theme in Theme::BUILT_IN {
            for role in PALETTE {
                for state in [VisualState::NONE, VisualState::DISABLED] {
                    let (disc, content) = interactive_pair(&theme, role, state);
                    let ratio = contrast_x100(disc, content);
                    assert!(
                        ratio >= AA_LARGE,
                        "{} {role:?} {state:?}: initials are {ratio}, floor is {AA_LARGE}",
                        theme.name
                    );
                }
            }
        }
    }
}
