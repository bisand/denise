//! Semantic theming.
//!
//! The role vocabulary is borrowed from [daisyUI](https://daisyui.com), which got
//! the important part right: a widget never names a colour, it names a *role*, and
//! every surface role has a **content** partner guaranteed to be readable on it.
//! Swapping a theme then cannot produce unreadable text, because readability is a
//! property of the pair rather than of the widget.
//!
//! ```
//! use denise::theme::{Radius, Role, Theme};
//!
//! let theme = Theme::DARK;
//! let (background, foreground) = theme.pair(Role::Primary);
//! let corner = theme.radius(Radius::Field);
//! # let _ = (background, foreground, corner);
//! ```
//!
//! A `Theme` is plain data with no global state. It travels explicitly — in the
//! draw context from M3 — so two displays on one device can run different themes,
//! and so nothing in the render hot path has to reach for shared mutable state.
//!
//! # What was not borrowed
//!
//! - **OKLCH storage.** daisyUI keeps colours in OKLCH, which is why its ramps stay
//!   perceptually even. Cube roots mean floating point, which means `libm` on
//!   `no_std` and output that is no longer bit-identical across architectures.
//!   Colours are stored as sRGB and derived with integers instead. A theme author
//!   who wants an OKLCH-even ramp can compute it elsewhere and set the values
//!   explicitly.
//! - **`--noise`.** A per-pixel texture makes every pixel differ from its
//!   neighbour, so a damaged region can never be repainted without a visible seam
//!   against the region next to it. It converts every frame into a full repaint,
//!   which is the one thing this project exists not to do.
//! - **Thirty-five built-in themes.** Three ship: [`Theme::LIGHT`],
//!   [`Theme::DARK`], and [`Theme::HIGH_CONTRAST`]. On a device that boots from
//!   flash, an unused theme is bytes that had to be paid for.
//!
//! [`--depth`](Theme::depth) survives as a number rather than a shadow. A real
//! blur is expensive in software, and worse, it spills outside the widget's bounds,
//! so every damage rectangle would have to be inflated by the blur radius.

mod contrast;

pub use contrast::{
    AA, AA_LARGE, AAA, contrast_x100, derive_content, derive_content_for, luminance,
};

use crate::color::Color;

/// Whether a theme is built around a light or a dark surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorScheme {
    /// Dark content on light surfaces.
    Light,
    /// Light content on dark surfaces.
    Dark,
}

/// A semantic colour slot.
///
/// Surface roles come in pairs with a `*Content` role; [`Role::content`] maps one
/// to the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum Role {
    /// The main surface: page and panel background.
    Base100,
    /// A slightly recessed surface, for wells and stripes.
    Base200,
    /// The most recessed surface, for borders and dividers.
    Base300,
    /// Text and icons on any `Base*` surface.
    BaseContent,

    /// The primary brand colour, for the main action.
    Primary,
    /// Content on [`Role::Primary`].
    PrimaryContent,
    /// A supporting brand colour.
    Secondary,
    /// Content on [`Role::Secondary`].
    SecondaryContent,
    /// An emphasis colour, for highlights and selection.
    Accent,
    /// Content on [`Role::Accent`].
    AccentContent,
    /// An unobtrusive colour, for secondary controls.
    Neutral,
    /// Content on [`Role::Neutral`].
    NeutralContent,

    /// Informational status.
    Info,
    /// Content on [`Role::Info`].
    InfoContent,
    /// Success status.
    Success,
    /// Content on [`Role::Success`].
    SuccessContent,
    /// Warning status.
    Warning,
    /// Content on [`Role::Warning`].
    WarningContent,
    /// Error status.
    Error,
    /// Content on [`Role::Error`].
    ErrorContent,
}

impl Role {
    /// Number of roles in a theme.
    pub const COUNT: usize = 20;

    /// The surface roles, each of which has a distinct content partner.
    pub const SURFACES: [Role; 11] = [
        Role::Base100,
        Role::Base200,
        Role::Base300,
        Role::Primary,
        Role::Secondary,
        Role::Accent,
        Role::Neutral,
        Role::Info,
        Role::Success,
        Role::Warning,
        Role::Error,
    ];

    /// The role to draw *on top of* this one.
    ///
    /// All three `Base*` surfaces share [`Role::BaseContent`], as in daisyUI: they
    /// are elevation steps of one surface, not three unrelated colours. A content
    /// role maps to itself.
    pub const fn content(self) -> Role {
        match self {
            Role::Base100 | Role::Base200 | Role::Base300 => Role::BaseContent,
            Role::Primary => Role::PrimaryContent,
            Role::Secondary => Role::SecondaryContent,
            Role::Accent => Role::AccentContent,
            Role::Neutral => Role::NeutralContent,
            Role::Info => Role::InfoContent,
            Role::Success => Role::SuccessContent,
            Role::Warning => Role::WarningContent,
            Role::Error => Role::ErrorContent,
            other => other,
        }
    }

    /// Returns `true` if this role is drawn on top of another rather than beneath.
    pub const fn is_content(self) -> bool {
        matches!(
            self,
            Role::BaseContent
                | Role::PrimaryContent
                | Role::SecondaryContent
                | Role::AccentContent
                | Role::NeutralContent
                | Role::InfoContent
                | Role::SuccessContent
                | Role::WarningContent
                | Role::ErrorContent
        )
    }
}

/// Which corner radius a widget class uses.
///
/// Three, not one per widget. A single token makes every control the same shape;
/// one per widget means every widget invents its own constant and the set drifts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Radius {
    /// Checkboxes, radios, toggles, badges.
    Selector,
    /// Buttons, inputs, selects, tabs.
    Field,
    /// Cards, dialogs, alerts, panels.
    Box,
}

/// Geometry tokens, in logical pixels at scale factor `1.0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Metrics {
    /// Corner radius for [`Radius::Selector`].
    pub radius_selector: i32,
    /// Corner radius for [`Radius::Field`].
    pub radius_field: i32,
    /// Corner radius for [`Radius::Box`].
    pub radius_box: i32,
    /// Edge length of a checkbox or radio.
    pub size_selector: i32,
    /// Height of a button or input.
    pub size_field: i32,
    /// Default border thickness.
    pub border: i32,
}

impl Metrics {
    /// Mouse-driven defaults.
    pub const DEFAULT: Self = Self {
        radius_selector: 8,
        radius_field: 6,
        radius_box: 12,
        size_selector: 20,
        size_field: 36,
        border: 1,
    };

    /// Touch defaults, sized for a finger on a panel rather than a cursor.
    ///
    /// 44 logical pixels is the smallest target most guidance considers reliable,
    /// and an industrial panel is often operated with gloves on.
    pub const TOUCH: Self = Self {
        radius_selector: 10,
        radius_field: 8,
        radius_box: 16,
        size_selector: 28,
        size_field: 48,
        border: 2,
    };

    /// These metrics in physical pixels at `scale`.
    ///
    /// Rounds without `f32::round`, which lives in `std`.
    pub fn scaled(self, scale: f32) -> Metrics {
        let s = |v: i32| ((v as f32) * scale + 0.5) as i32;
        Metrics {
            radius_selector: s(self.radius_selector),
            radius_field: s(self.radius_field),
            radius_box: s(self.radius_box),
            size_selector: s(self.size_selector),
            size_field: s(self.size_field),
            border: s(self.border).max(1),
        }
    }

    /// The radius for a widget class.
    pub const fn radius(self, which: Radius) -> i32 {
        match which {
            Radius::Selector => self.radius_selector,
            Radius::Field => self.radius_field,
            Radius::Box => self.radius_box,
        }
    }
}

/// A surface whose content partner does not have enough contrast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContrastFailure {
    /// The surface role.
    pub surface: Role,
    /// Its content role.
    pub content: Role,
    /// The ratio achieved, times 100.
    pub ratio_x100: u32,
    /// The ratio required, times 100.
    pub required: u32,
}

/// A complete set of colours and geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    /// Human-readable name, for diagnostics and theme pickers.
    pub name: &'static str,
    /// Whether this theme is light or dark.
    pub scheme: ColorScheme,
    /// Geometry tokens.
    pub metrics: Metrics,
    /// Elevation emphasis, `0` for flat.
    ///
    /// Not a blur. A widget honours this by lightening its top edge and darkening
    /// its bottom edge by roughly this much, which stays inside its own bounds and
    /// so costs the damage tracker nothing.
    pub depth: u8,
    colors: [Color; Role::COUNT],
}

impl Theme {
    /// Builds a theme from nine seed colours, deriving the rest.
    ///
    /// The two recessed base surfaces and all nine content colours are computed.
    /// This is a `const fn`, so the built-in themes cost nothing at runtime and
    /// cannot drift out of step with the derivation rules the way a hand-written
    /// table would.
    #[allow(clippy::too_many_arguments)]
    pub const fn from_seeds(
        name: &'static str,
        scheme: ColorScheme,
        base: Color,
        primary: Color,
        secondary: Color,
        accent: Color,
        neutral: Color,
        info: Color,
        success: Color,
        warning: Color,
        error: Color,
    ) -> Theme {
        Theme::from_seeds_at(
            name, scheme, AA, base, primary, secondary, accent, neutral, info, success, warning,
            error,
        )
    }

    /// [`Theme::from_seeds`] at a chosen contrast target.
    ///
    /// Derivation stops at the *first* mix that clears the target, so a theme built
    /// at [`AA`] will generally not also satisfy [`AAA`]. A theme that exists to be
    /// legible in glare should say so here rather than hope.
    #[allow(clippy::too_many_arguments)]
    pub const fn from_seeds_at(
        name: &'static str,
        scheme: ColorScheme,
        target: u32,
        base: Color,
        primary: Color,
        secondary: Color,
        accent: Color,
        neutral: Color,
        info: Color,
        success: Color,
        warning: Color,
        error: Color,
    ) -> Theme {
        // Recessed surfaces step darker, in both schemes — that is what daisyUI
        // does and what reads as depth. Below a floor there is no darker left, so
        // an OLED-black theme steps lighter instead.
        let toward = if luminance(base) < 3000 {
            Color::WHITE
        } else {
            Color::BLACK
        };
        let base_200 = base.mix(toward, 18);
        let base_300 = base.mix(toward, 36);

        Theme {
            name,
            scheme,
            metrics: Metrics::DEFAULT,
            depth: 0,
            colors: [
                base,
                base_200,
                base_300,
                // Against all three base surfaces, not just the main one.
                derive_content_for(&[base, base_200, base_300], target),
                primary,
                derive_content(primary, target),
                secondary,
                derive_content(secondary, target),
                accent,
                derive_content(accent, target),
                neutral,
                derive_content(neutral, target),
                info,
                derive_content(info, target),
                success,
                derive_content(success, target),
                warning,
                derive_content(warning, target),
                error,
                derive_content(error, target),
            ],
        }
    }

    /// The colour for a role.
    #[inline]
    pub const fn color(self, role: Role) -> Color {
        self.colors[role as usize]
    }

    /// The colour to draw on top of `role`.
    #[inline]
    pub const fn content_of(self, role: Role) -> Color {
        self.colors[role.content() as usize]
    }

    /// A surface and its readable foreground, in that order.
    #[inline]
    pub const fn pair(self, role: Role) -> (Color, Color) {
        (self.color(role), self.content_of(role))
    }

    /// The corner radius for a widget class, in logical pixels.
    #[inline]
    pub const fn radius(self, which: Radius) -> i32 {
        self.metrics.radius(which)
    }

    /// This theme with different geometry.
    pub const fn with_metrics(mut self, metrics: Metrics) -> Theme {
        self.metrics = metrics;
        self
    }

    /// This theme with its geometry at `scale` — the one theme call a
    /// scale-aware application makes at construction.
    ///
    /// Only the metrics scale; colours have no size. The layout rectangles and
    /// text sizes are the application's own numbers and scale through
    /// [`Rect::scaled`](crate::Rect::scaled) and `TextStyle::scaled` in the
    /// same one place. See `docs/design.md` for why the application, not the
    /// tree, does the multiplying.
    pub fn scaled(self, scale: f32) -> Theme {
        let metrics = self.metrics.scaled(scale);
        self.with_metrics(metrics)
    }

    /// This theme with a different elevation emphasis.
    pub const fn with_depth(mut self, depth: u8) -> Theme {
        self.depth = depth;
        self
    }

    /// This theme with one role overridden.
    ///
    /// Overriding a surface does **not** re-derive its content partner; override
    /// that too, or run [`Theme::validate`].
    pub const fn with_color(mut self, role: Role, color: Color) -> Theme {
        self.colors[role as usize] = color;
        self
    }

    /// Checks every surface against its content partner at `target`.
    ///
    /// Returns the worst offender. Worth calling at startup on a hand-written
    /// theme: grey-on-grey discovered on a factory floor is expensive, and
    /// discovered in a unit test is free.
    pub fn validate(self, target: u32) -> Result<(), ContrastFailure> {
        let mut worst: Option<ContrastFailure> = None;
        for surface in Role::SURFACES {
            let ratio = contrast_x100(self.color(surface), self.content_of(surface));
            if ratio < target {
                let failure = ContrastFailure {
                    surface,
                    content: surface.content(),
                    ratio_x100: ratio,
                    required: target,
                };
                if worst.is_none_or(|w| ratio < w.ratio_x100) {
                    worst = Some(failure);
                }
            }
        }
        match worst {
            Some(f) => Err(f),
            None => Ok(()),
        }
    }
}

/// Catppuccin Mocha, near enough.
pub const DARK: Theme = Theme::from_seeds(
    "dark",
    ColorScheme::Dark,
    Color::from_rgb888(0x1E1E2E),
    Color::from_rgb888(0x89B4FA),
    Color::from_rgb888(0xF5C2E7),
    Color::from_rgb888(0x94E2D5),
    Color::from_rgb888(0x585B70),
    Color::from_rgb888(0x89DCEB),
    Color::from_rgb888(0xA6E3A1),
    Color::from_rgb888(0xF9E2AF),
    Color::from_rgb888(0xF38BA8),
);

/// Catppuccin Latte, near enough.
pub const LIGHT: Theme = Theme::from_seeds(
    "light",
    ColorScheme::Light,
    Color::from_rgb888(0xEFF1F5),
    Color::from_rgb888(0x1E66F5),
    Color::from_rgb888(0xEA76CB),
    Color::from_rgb888(0x179299),
    Color::from_rgb888(0x6C6F85),
    Color::from_rgb888(0x209FB5),
    Color::from_rgb888(0x40A02B),
    Color::from_rgb888(0xDF8E1D),
    Color::from_rgb888(0xD20F39),
);

/// Maximum-contrast theme for panels read in direct sunlight, or through a visor.
///
/// Saturated primaries on black. Not pretty; legible at an angle, in glare, by
/// someone wearing safety glasses. Defaults to touch metrics, because the machine
/// this ends up on is not operated with a mouse.
pub const HIGH_CONTRAST: Theme = Theme::from_seeds_at(
    "high-contrast",
    ColorScheme::Dark,
    AAA,
    Color::from_rgb888(0x000000),
    Color::from_rgb888(0xFFFF00),
    Color::from_rgb888(0x00FFFF),
    // Lightened from pure magenta and pure red: against black, both top out around
    // 6.7:1 and cannot reach AAA. `validate` caught this, which is the argument for
    // having it.
    Color::from_rgb888(0xFF80FF),
    Color::from_rgb888(0xC0C0C0),
    Color::from_rgb888(0x00FFFF),
    Color::from_rgb888(0x00FF00),
    Color::from_rgb888(0xFFFF00),
    Color::from_rgb888(0xFF8080),
)
.with_metrics(Metrics::TOUCH);

impl Theme {
    /// See [`DARK`].
    pub const DARK: Theme = DARK;
    /// See [`LIGHT`].
    pub const LIGHT: Theme = LIGHT;
    /// See [`HIGH_CONTRAST`].
    pub const HIGH_CONTRAST: Theme = HIGH_CONTRAST;

    /// Every theme that ships with Denise.
    pub const BUILT_IN: [Theme; 3] = [LIGHT, DARK, HIGH_CONTRAST];
}

impl Default for Theme {
    fn default() -> Self {
        DARK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_themes_are_readable() {
        for theme in Theme::BUILT_IN {
            if let Err(f) = theme.validate(AA) {
                panic!(
                    "theme {:?}: {:?} on {:?} is {}:1, needs {}:1",
                    theme.name,
                    f.content,
                    f.surface,
                    f.ratio_x100 as f32 / 100.0,
                    f.required as f32 / 100.0
                );
            }
        }
    }

    #[test]
    fn high_contrast_clears_the_strictest_target() {
        // The whole reason it exists. If it only scrapes AA it is not doing its job.
        HIGH_CONTRAST
            .validate(AAA)
            .expect("high-contrast must clear AAA");
    }

    #[test]
    fn every_role_maps_to_its_own_slot() {
        // A duplicated discriminant would silently alias two roles to one colour.
        let mut seen = [false; Role::COUNT];
        for surface in Role::SURFACES {
            for role in [surface, surface.content()] {
                let i = role as usize;
                assert!(i < Role::COUNT);
                seen[i] = true;
            }
        }
        assert!(
            seen.iter().all(|&s| s),
            "a role is unreachable via SURFACES"
        );
    }

    #[test]
    fn content_roles_are_their_own_content() {
        for surface in Role::SURFACES {
            let content = surface.content();
            assert!(content.is_content(), "{surface:?} paired with a surface");
            assert_eq!(content.content(), content);
        }
    }

    #[test]
    fn the_three_base_surfaces_share_one_content_colour() {
        for base in [Role::Base100, Role::Base200, Role::Base300] {
            assert_eq!(base.content(), Role::BaseContent);
        }
    }

    #[test]
    fn recessed_surfaces_are_distinct_from_the_main_one() {
        for theme in Theme::BUILT_IN {
            let (b1, b2, b3) = (
                theme.color(Role::Base100),
                theme.color(Role::Base200),
                theme.color(Role::Base300),
            );
            assert_ne!(b1, b2, "{}: base-200 collapsed onto base-100", theme.name);
            assert_ne!(b2, b3, "{}: base-300 collapsed onto base-200", theme.name);
        }
    }

    #[test]
    fn a_black_base_ramps_lighter_rather_than_nowhere() {
        // There is nothing darker than black; the ramp has to reverse or vanish.
        let t = HIGH_CONTRAST;
        assert!(luminance(t.color(Role::Base200)) > luminance(t.color(Role::Base100)));
        assert!(luminance(t.color(Role::Base300)) > luminance(t.color(Role::Base200)));
    }

    #[test]
    fn a_light_base_ramps_darker() {
        let t = LIGHT;
        assert!(luminance(t.color(Role::Base200)) < luminance(t.color(Role::Base100)));
        assert!(luminance(t.color(Role::Base300)) < luminance(t.color(Role::Base200)));
    }

    #[test]
    fn validate_catches_a_deliberately_broken_theme() {
        let broken = DARK.with_color(Role::PrimaryContent, DARK.color(Role::Primary));
        let failure = broken.validate(AA).expect_err("must be caught");
        assert_eq!(failure.surface, Role::Primary);
        assert_eq!(failure.ratio_x100, 100);
    }

    #[test]
    fn validate_reports_the_worst_offender() {
        let broken = DARK
            .with_color(Role::InfoContent, DARK.color(Role::Info))
            .with_color(Role::ErrorContent, Color::from_rgb888(0x808080));
        let failure = broken.validate(AA).expect_err("must be caught");
        assert_eq!(failure.surface, Role::Info, "should report the 1:1 pair");
    }

    #[test]
    fn metrics_scale_and_never_lose_the_border() {
        let m = Metrics::DEFAULT.scaled(2.0);
        assert_eq!(m.size_field, 72);
        assert_eq!(m.radius_box, 24);
        // A hairline border must survive rounding down; a border of zero is a
        // widget with no edge, which reads as a rendering fault.
        assert_eq!(Metrics::DEFAULT.scaled(0.1).border, 1);
    }

    #[test]
    fn touch_targets_are_large_enough_to_hit() {
        // A compile-time check: shrinking this below the reliable-hit threshold
        // should fail the build, not a test run.
        const { assert!(Metrics::TOUCH.size_field >= 44) };
    }

    #[test]
    fn pair_agrees_with_the_individual_lookups() {
        let (bg, fg) = DARK.pair(Role::Success);
        assert_eq!(bg, DARK.color(Role::Success));
        assert_eq!(fg, DARK.color(Role::SuccessContent));
    }

    #[test]
    fn builders_are_const_usable() {
        const CUSTOM: Theme = DARK.with_depth(3).with_metrics(Metrics::TOUCH);
        assert_eq!(CUSTOM.depth, 3);
        assert_eq!(CUSTOM.radius(Radius::Box), Metrics::TOUCH.radius_box);
    }
}
