//! Integer WCAG contrast.
//!
//! Used to derive readable content colours and to check hand-written themes. All
//! integer, like the rest of the drawing path: a theme must evaluate identically on
//! the developer's machine and on the panel, and `f32::powf` is not available in a
//! `no_std` build without pulling in `libm`.

use crate::color::Color;

/// sRGB 8-bit to linear light, scaled to `0..=65535`.
///
/// Generated from the sRGB transfer function — the piecewise one with the linear
/// segment below 0.04045, not the `x^2.2` approximation, which is off by enough
/// near black to change a contrast verdict.
const SRGB_TO_LINEAR: [u16; 256] = [
    0, 20, 40, 60, 80, 99, 119, 139, 159, 179, 199, 219, 241, 264, 288, 313, 340, 367, 396, 427,
    458, 491, 526, 562, 599, 637, 677, 718, 761, 805, 851, 898, 947, 997, 1048, 1101, 1156, 1212,
    1270, 1330, 1391, 1453, 1517, 1583, 1651, 1720, 1790, 1863, 1937, 2013, 2090, 2170, 2250, 2333,
    2418, 2504, 2592, 2681, 2773, 2866, 2961, 3058, 3157, 3258, 3360, 3464, 3570, 3678, 3788, 3900,
    4014, 4129, 4247, 4366, 4488, 4611, 4736, 4864, 4993, 5124, 5257, 5392, 5530, 5669, 5810, 5953,
    6099, 6246, 6395, 6547, 6700, 6856, 7014, 7174, 7335, 7500, 7666, 7834, 8004, 8177, 8352, 8528,
    8708, 8889, 9072, 9258, 9445, 9635, 9828, 10022, 10219, 10417, 10619, 10822, 11028, 11235,
    11446, 11658, 11873, 12090, 12309, 12530, 12754, 12980, 13209, 13440, 13673, 13909, 14146,
    14387, 14629, 14874, 15122, 15371, 15623, 15878, 16135, 16394, 16656, 16920, 17187, 17456,
    17727, 18001, 18277, 18556, 18837, 19121, 19407, 19696, 19987, 20281, 20577, 20876, 21177,
    21481, 21787, 22096, 22407, 22721, 23038, 23357, 23678, 24002, 24329, 24658, 24990, 25325,
    25662, 26001, 26344, 26688, 27036, 27386, 27739, 28094, 28452, 28813, 29176, 29542, 29911,
    30282, 30656, 31033, 31412, 31794, 32179, 32567, 32957, 33350, 33745, 34143, 34544, 34948,
    35355, 35764, 36176, 36591, 37008, 37429, 37852, 38278, 38706, 39138, 39572, 40009, 40449,
    40891, 41337, 41785, 42236, 42690, 43147, 43606, 44069, 44534, 45002, 45473, 45947, 46423,
    46903, 47385, 47871, 48359, 48850, 49344, 49841, 50341, 50844, 51349, 51858, 52369, 52884,
    53401, 53921, 54445, 54971, 55500, 56032, 56567, 57105, 57646, 58190, 58737, 59287, 59840,
    60396, 60955, 61517, 62082, 62650, 63221, 63795, 64372, 64952, 65535,
];

/// `0.05` in the same fixed-point scale as [`luminance`], the WCAG flare term.
const FLARE: u32 = 3277;

/// Relative luminance per WCAG 2.x, scaled to `0..=65535`.
pub const fn luminance(color: Color) -> u32 {
    let r = SRGB_TO_LINEAR[color.r as usize] as u32;
    let g = SRGB_TO_LINEAR[color.g as usize] as u32;
    let b = SRGB_TO_LINEAR[color.b as usize] as u32;
    (2126 * r + 7152 * g + 722 * b) / 10000
}

/// Contrast ratio between two colours, times 100.
///
/// `100` is identical, `2100` is black on white. WCAG AA body text wants `450`,
/// AA large text `300`, AAA `700`.
pub const fn contrast_x100(a: Color, b: Color) -> u32 {
    let la = luminance(a) + FLARE;
    let lb = luminance(b) + FLARE;
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    // Rounded, so black on white reads as the canonical 21.00 rather than 20.99.
    (hi * 100 + lo / 2) / lo
}

/// The WCAG AA threshold for body text, times 100.
pub const AA: u32 = 450;
/// The WCAG AA threshold for large text, times 100.
pub const AA_LARGE: u32 = 300;
/// The WCAG AAA threshold for body text, times 100.
pub const AAA: u32 = 700;

/// Derives a readable foreground for `background`, keeping its hue.
///
/// Walks from the background colour towards black or white — whichever direction
/// has more headroom — and stops at the first mix that clears `target`. Stopping
/// early rather than jumping to pure black or white is what keeps a derived theme
/// looking like a theme instead of a wireframe.
///
/// A narrow band of mid-tones (relative luminance about 0.175 to 0.183) cannot
/// reach 4.5:1 against *either* extreme. For those this returns the better of the
/// two and [`Theme::validate`](crate::theme::Theme::validate) reports the shortfall
/// rather than pretending.
pub const fn derive_content(background: Color, target: u32) -> Color {
    derive_content_for(&[background], target)
}

/// Derives one foreground readable over *every* background given.
///
/// The direction is chosen from `backgrounds[0]`, then the mix walks until it
/// clears `target` against all of them. Deriving against only the main surface is
/// the obvious bug here: `base-content` also has to be legible on `base-200` and
/// `base-300`, and in a light theme those are darker, so they are strictly harder
/// than the surface the colour was picked for.
pub const fn derive_content_for(backgrounds: &[Color], target: u32) -> Color {
    let primary = backgrounds[0];

    // Pick the direction by what it actually achieves, not by whether the surface
    // is "light". The crossover where black and white contrast equally sits at a
    // relative luminance of about 0.179, nowhere near the midpoint — a mid-amber
    // looks light, is below 0.5, and yet only black is readable on it.
    let toward = if contrast_x100(Color::BLACK, primary) >= contrast_x100(Color::WHITE, primary) {
        Color::BLACK
    } else {
        Color::WHITE
    };

    let mut t: u16 = 0;
    while t <= 255 {
        let candidate = primary.mix(toward, t as u8);
        let mut i = 0;
        let mut ok = true;
        while i < backgrounds.len() {
            if contrast_x100(candidate, backgrounds[i]) < target {
                ok = false;
                break;
            }
            i += 1;
        }
        if ok {
            return candidate;
        }
        t += 13;
    }

    toward
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_on_white_is_the_maximum_ratio() {
        assert_eq!(contrast_x100(Color::BLACK, Color::WHITE), 2100);
    }

    #[test]
    fn a_colour_has_no_contrast_with_itself() {
        let c = Color::from_rgb888(0x89B4FA);
        assert_eq!(contrast_x100(c, c), 100);
    }

    #[test]
    fn contrast_is_symmetric() {
        let a = Color::from_rgb888(0x1E1E2E);
        let b = Color::from_rgb888(0xCDD6F4);
        assert_eq!(contrast_x100(a, b), contrast_x100(b, a));
    }

    #[test]
    fn luminance_is_ordered_and_spans_the_range() {
        assert_eq!(luminance(Color::BLACK), 0);
        assert_eq!(luminance(Color::WHITE), 65535);
        assert!(luminance(Color::from_rgb888(0x808080)) < luminance(Color::WHITE));
        assert!(luminance(Color::from_rgb888(0x808080)) > luminance(Color::BLACK));
        // Green carries most of the luminance, blue almost none.
        assert!(luminance(Color::rgb(0, 255, 0)) > luminance(Color::rgb(255, 0, 0)));
        assert!(luminance(Color::rgb(255, 0, 0)) > luminance(Color::rgb(0, 0, 255)));
    }

    #[test]
    fn known_wcag_pairs_match_the_published_ratios() {
        // Spot values from the WCAG contrast formula, to within rounding.
        let grey = Color::from_rgb888(0x767676);
        assert!((449..=460).contains(&contrast_x100(grey, Color::WHITE)));
        let blue = Color::from_rgb888(0x0000FF);
        assert!((855..=865).contains(&contrast_x100(blue, Color::WHITE)));
    }

    #[test]
    fn derived_content_is_readable_over_any_background() {
        // Sweep the whole cube coarsely. Every background must either get a passing
        // foreground or be one of the mid-tones that provably cannot have one.
        let mut impossible = 0;
        for r in (0..=255).step_by(17) {
            for g in (0..=255).step_by(17) {
                for b in (0..=255).step_by(17) {
                    let bg = Color::rgb(r as u8, g as u8, b as u8);
                    let fg = derive_content(bg, AA);
                    if contrast_x100(fg, bg) < AA {
                        // Only the known dead band may fail, and only against both
                        // extremes at once.
                        assert!(
                            contrast_x100(Color::BLACK, bg) < AA
                                && contrast_x100(Color::WHITE, bg) < AA,
                            "{bg:?} had a readable option but derive_content missed it"
                        );
                        impossible += 1;
                    }
                }
            }
        }
        // The dead band is real but must stay tiny; if this balloons, the search
        // has broken rather than the maths.
        assert!(impossible < 60, "{impossible} backgrounds failed");
    }

    #[test]
    fn derived_content_keeps_the_hue() {
        // A blue surface should get a blue-tinted foreground, not flat black.
        let bg = Color::from_rgb888(0x1E3A8A);
        let fg = derive_content(bg, AA);
        assert!(
            fg.b >= fg.r,
            "derived foreground lost the blue cast: {fg:?}"
        );
    }

    #[test]
    fn stricter_targets_give_stronger_contrast() {
        let bg = Color::from_rgb888(0x313244);
        let aa = contrast_x100(derive_content(bg, AA), bg);
        let aaa = contrast_x100(derive_content(bg, AAA), bg);
        assert!(aa >= AA);
        assert!(aaa >= aa);
    }
}
