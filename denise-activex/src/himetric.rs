//! OLE's unit of length, and back.
//!
//! OLE measures extents in HIMETRIC — hundredths of a millimetre — and everything
//! else in this project measures pixels, so this conversion sits on every extent
//! that crosses the boundary.
//!
//! Deliberately outside `cfg(windows)`. It is arithmetic, it is the part that
//! goes wrong, and a Windows CI runner is a slow place to find that out — which
//! is exactly how the first version was caught. The same split `denise-win32`
//! makes for its keymap, for the same reason.

/// Hundredths of a millimetre to the inch.
const HIMETRIC_PER_INCH: i64 = 2540;
/// Pixels to the inch, at the 96 DPI OLE assumes.
const PIXELS_PER_INCH: i64 = 96;

/// Converts pixels to the hundredths of a millimetre OLE measures extents in.
///
/// The ratio is 2540/96 — 26.458… — so it is emphatically **not** a constant to
/// precompute. `const PER_PIXEL: i32 = 2540 / 96` is 26, and every extent the
/// control reported would have been 1.7% short: a form editor would draw it
/// slightly too small and nothing would ever say why. Multiply first, divide
/// second, and round rather than truncate so the two directions round-trip.
///
/// Widened to `i64` because `pixels * 2540` overflows an `i32` above about
/// 845,000, and the argument comes from a container.
pub fn pixels_to_himetric(pixels: i32) -> i32 {
    scale(pixels as i64, HIMETRIC_PER_INCH, PIXELS_PER_INCH)
}

/// And back.
pub fn himetric_to_pixels(himetric: i32) -> i32 {
    scale(himetric as i64, PIXELS_PER_INCH, HIMETRIC_PER_INCH)
}

/// `value * numerator / denominator`, rounded to nearest and saturating.
///
/// Rounding is what makes the two conversions inverses at small sizes: one pixel
/// is 26.458 units, and truncating both ways turns it back into zero.
fn scale(value: i64, numerator: i64, denominator: i64) -> i32 {
    let half = denominator / 2;
    let scaled = if value >= 0 {
        (value * numerator + half) / denominator
    } else {
        (value * numerator - half) / denominator
    };
    scaled.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// OLE measures in hundredths of a millimetre and everything else measures in
    /// pixels, so this conversion sits on every extent that crosses the boundary.
    ///
    /// This caught the version that precomputed `2540 / 96` as an integer
    /// constant — 26 rather than 26.458 — which made every extent 1.7% short. A
    /// container would have drawn the control slightly too small forever and
    /// nothing would have pointed at a constant.
    #[test]
    fn himetric_round_trips_at_the_sizes_a_control_uses() {
        for pixels in [1, 2, 96, 100, 200, 640, 1080, 1920] {
            assert_eq!(
                himetric_to_pixels(pixels_to_himetric(pixels)),
                pixels,
                "{pixels} px did not survive the round trip"
            );
        }
    }

    /// The definition, not an approximation of it: 96 pixels is one inch, and one
    /// inch is 2540 hundredths of a millimetre.
    #[test]
    fn one_inch_is_exact_in_both_directions() {
        assert_eq!(pixels_to_himetric(96), 2540);
        assert_eq!(himetric_to_pixels(2540), 96);
        assert_eq!(pixels_to_himetric(192), 5080);
    }

    /// The argument comes from a container, so it can be anything. `pixels * 2540`
    /// overflows an `i32` above about 845,000, and a panic inside a COM method is
    /// worse than a clamped answer.
    #[test]
    fn an_absurd_extent_saturates_rather_than_overflowing() {
        assert_eq!(pixels_to_himetric(i32::MAX), i32::MAX);
        assert_eq!(pixels_to_himetric(i32::MIN), i32::MIN);
        assert_eq!(himetric_to_pixels(i32::MAX), 81_164_736);
    }
}
