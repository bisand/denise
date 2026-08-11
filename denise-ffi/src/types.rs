//! The `#[repr(C)]` half of the ABI, and the numbers either side of it.
//!
//! Every Rust type Denise exposes here is an enum whose discriminants are an
//! implementation detail. None of them may leak: an ABI that changes when
//! somebody reorders a `match` is not an ABI. So each one gets an explicit table,
//! and each table is checked against `denise.h`.

use denise::{Modifiers, PixelFormat, PointerButton, Rect, Role, Theme};

/// A rectangle, laid out for C. Matches `DeniseRect` in `denise.h`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeniseRect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Extent to the right.
    pub width: i32,
    /// Extent downwards.
    pub height: i32,
}

impl From<Rect> for DeniseRect {
    fn from(r: Rect) -> Self {
        Self {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        }
    }
}

impl From<DeniseRect> for Rect {
    fn from(r: DeniseRect) -> Self {
        Rect::new(r.x, r.y, r.width, r.height)
    }
}

/// A pixel buffer the host owns, described for one call to
/// [`denise_ui_paint`](crate::denise_ui_paint).
///
/// This is the whole seam between Denise and a host that already has a window.
/// Denise never allocates the buffer, never presents it, and never remembers it —
/// which is what lets the same library serve a Win32 `HDC`, an `NSView`'s backing
/// store and a WinForms control without knowing which it is.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DeniseFrame {
    /// First word of the buffer. `0xAARRGGBB` in native byte order.
    pub pixels: *mut u32,
    /// Length of the buffer in **words**, not bytes.
    pub len: usize,
    /// Visible width in pixels.
    pub width: u32,
    /// Visible height in pixels.
    pub height: u32,
    /// Distance between the starts of consecutive rows, in **pixels**.
    ///
    /// May exceed `width`. A host that assumes rows are contiguous works on a
    /// desktop and shears on a pitch-aligned framebuffer.
    pub stride: u32,
    /// [`DENISE_FORMAT_ARGB8888`] or [`DENISE_FORMAT_XRGB8888`].
    pub format: u32,
    /// How many presents ago this buffer's contents are from; negative for
    /// "undefined, repaint everything".
    ///
    /// Modelled on `EGL_EXT_buffer_age`. A host that double-buffers and reports
    /// `1` here every frame will show stale content on alternate frames; report
    /// what is true, or report `-1` and pay for a full repaint.
    pub buffer_age: i32,
}

/// `0xAARRGGBB`, alpha honoured.
pub const DENISE_FORMAT_ARGB8888: u32 = 0;
/// `0xXXRRGGBB`, high byte ignored.
pub const DENISE_FORMAT_XRGB8888: u32 = 1;

/// Maps [`DeniseFrame::format`] onto the core's enum.
pub fn pixel_format(value: u32) -> Option<PixelFormat> {
    match value {
        DENISE_FORMAT_ARGB8888 => Some(PixelFormat::Argb8888),
        DENISE_FORMAT_XRGB8888 => Some(PixelFormat::Xrgb8888),
        _ => None,
    }
}

/// The light built-in theme.
pub const DENISE_THEME_LIGHT: u32 = 0;
/// The dark built-in theme.
pub const DENISE_THEME_DARK: u32 = 1;
/// The high-contrast built-in theme.
pub const DENISE_THEME_HIGH_CONTRAST: u32 = 2;

/// Maps a theme number onto one of [`Theme::BUILT_IN`].
pub fn theme(value: u32) -> Option<Theme> {
    Theme::BUILT_IN.get(value as usize).copied()
}

/// No role at all: a panel with no fill, or no border.
pub const DENISE_ROLE_NONE: i32 = -1;

macro_rules! role_table {
    ($( $c_name:ident = $value:literal => $variant:ident ),* $(,)?) => {
        $(
            #[doc = concat!("[`Role::", stringify!($variant), "`] as an ABI number.")]
            pub const $c_name: i32 = $value;
        )*

        /// Maps a role number onto the core's enum, or `None` for
        /// [`DENISE_ROLE_NONE`] and anything else out of range.
        ///
        /// The order is [`Role`]'s own declaration order, which is also its
        /// `repr(usize)` discriminant — but written out rather than transmuted,
        /// so reordering the enum upstream is a compile error here instead of a
        /// silent recolouring of every host that ever linked this.
        pub fn role(value: i32) -> Option<Role> {
            Some(match value {
                $( $value => Role::$variant, )*
                _ => return None,
            })
        }

        /// Every role, as `(C name, value)`. Used by the header sync test.
        pub const ROLE_TABLE: &[(&str, i32)] = &[ $( (stringify!($c_name), $value) ),* ];
    };
}

role_table! {
    DENISE_ROLE_BASE_100 = 0 => Base100,
    DENISE_ROLE_BASE_200 = 1 => Base200,
    DENISE_ROLE_BASE_300 = 2 => Base300,
    DENISE_ROLE_BASE_CONTENT = 3 => BaseContent,
    DENISE_ROLE_PRIMARY = 4 => Primary,
    DENISE_ROLE_PRIMARY_CONTENT = 5 => PrimaryContent,
    DENISE_ROLE_SECONDARY = 6 => Secondary,
    DENISE_ROLE_SECONDARY_CONTENT = 7 => SecondaryContent,
    DENISE_ROLE_ACCENT = 8 => Accent,
    DENISE_ROLE_ACCENT_CONTENT = 9 => AccentContent,
    DENISE_ROLE_NEUTRAL = 10 => Neutral,
    DENISE_ROLE_NEUTRAL_CONTENT = 11 => NeutralContent,
    DENISE_ROLE_INFO = 12 => Info,
    DENISE_ROLE_INFO_CONTENT = 13 => InfoContent,
    DENISE_ROLE_SUCCESS = 14 => Success,
    DENISE_ROLE_SUCCESS_CONTENT = 15 => SuccessContent,
    DENISE_ROLE_WARNING = 16 => Warning,
    DENISE_ROLE_WARNING_CONTENT = 17 => WarningContent,
    DENISE_ROLE_ERROR = 18 => Error,
    DENISE_ROLE_ERROR_CONTENT = 19 => ErrorContent,
}

/// Primary pointer button.
pub const DENISE_BUTTON_LEFT: u32 = 0;
/// Secondary pointer button.
pub const DENISE_BUTTON_RIGHT: u32 = 1;
/// Wheel button.
pub const DENISE_BUTTON_MIDDLE: u32 = 2;
/// Set on any further button; the low bits carry the platform's own index.
pub const DENISE_BUTTON_OTHER: u32 = 0x100;

/// Maps a button number onto the core's enum.
pub fn button(value: u32) -> Option<PointerButton> {
    Some(match value {
        DENISE_BUTTON_LEFT => PointerButton::Left,
        DENISE_BUTTON_RIGHT => PointerButton::Right,
        DENISE_BUTTON_MIDDLE => PointerButton::Middle,
        v if v & DENISE_BUTTON_OTHER != 0 => PointerButton::Other((v & 0xFF) as u16),
        _ => return None,
    })
}

/// Either shift key.
pub const DENISE_MOD_SHIFT: u32 = 1 << 0;
/// Either control key.
pub const DENISE_MOD_CTRL: u32 = 1 << 1;
/// Either alt key.
pub const DENISE_MOD_ALT: u32 = 1 << 2;
/// Either super, meta or command key.
pub const DENISE_MOD_SUPER: u32 = 1 << 3;

/// Maps a modifier bitset onto the core's. Unknown bits are ignored rather than
/// rejected, so a host that learns a new modifier does not start failing calls.
pub fn modifiers(value: u32) -> Modifiers {
    let mut out = Modifiers::NONE;
    for (bit, modifier) in [
        (DENISE_MOD_SHIFT, Modifiers::SHIFT),
        (DENISE_MOD_CTRL, Modifiers::CTRL),
        (DENISE_MOD_ALT, Modifiers::ALT),
        (DENISE_MOD_SUPER, Modifiers::SUPER),
    ] {
        if value & bit != 0 {
            out |= modifier;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_role_number_is_a_distinct_role() {
        let mut seen = Vec::new();
        for value in 0..Role::COUNT as i32 {
            let role = role(value).unwrap_or_else(|| panic!("role {value} unmapped"));
            assert!(!seen.contains(&role), "{role:?} appears twice");
            seen.push(role);
        }
        assert_eq!(seen.len(), Role::COUNT);
        assert_eq!(role(Role::COUNT as i32), None);
        // -1 is how a caller says "no role at all" for an optional fill or border.
        assert_eq!(role(-1), None);
    }

    #[test]
    fn every_theme_number_is_a_distinct_theme() {
        for (value, expected) in Theme::BUILT_IN.iter().enumerate() {
            assert_eq!(theme(value as u32).as_ref(), Some(expected));
        }
        assert_eq!(theme(Theme::BUILT_IN.len() as u32), None);
    }

    #[test]
    fn buttons_round_trip_including_the_unnamed_ones() {
        assert_eq!(button(DENISE_BUTTON_LEFT), Some(PointerButton::Left));
        assert_eq!(button(DENISE_BUTTON_RIGHT), Some(PointerButton::Right));
        assert_eq!(button(DENISE_BUTTON_MIDDLE), Some(PointerButton::Middle));
        assert_eq!(
            button(DENISE_BUTTON_OTHER | 7),
            Some(PointerButton::Other(7))
        );
        assert_eq!(button(3), None);
    }

    #[test]
    fn modifier_bits_match_the_core() {
        let all = modifiers(DENISE_MOD_SHIFT | DENISE_MOD_CTRL | DENISE_MOD_ALT | DENISE_MOD_SUPER);
        assert!(all.contains(Modifiers::SHIFT | Modifiers::CTRL));
        assert!(all.contains(Modifiers::ALT | Modifiers::SUPER));
        assert!(modifiers(0).is_empty());
        // A bit this build does not know is not an error; it is just not a
        // modifier yet.
        assert!(modifiers(1 << 20).is_empty());
    }

    #[test]
    fn rectangles_survive_the_crossing() {
        let rect = Rect::new(-3, 7, 40, 900);
        assert_eq!(Rect::from(DeniseRect::from(rect)), rect);
    }
}
