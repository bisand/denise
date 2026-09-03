//! Binary-turn angles and the fixed point the geometry runs in.
//!
//! A full revolution is [`TURN`] = 65536, angle 0 points at twelve o'clock, and
//! positive sweeps go clockwise. This is the unit a panel actually computes in:
//! a progress ring is `done * TURN / total` with no floating point and no π, and
//! wrap-around is `& (TURN - 1)` rather than a comparison nobody remembers to
//! write. Radians would put `f32` and a libm dependency in the one crate that
//! has neither; degrees would make the common quarters 90/180/270 but leave the
//! progress ring with a rounding remainder. Negative sweeps go anticlockwise.
//!
//!
//! No floating point, so a `no_std` target needs no `libm` and the same angle
//! gives the same pixel on x86 and on ARM.

/// One full revolution, in the angle unit every arc call takes.
///
/// Angle 0 is twelve o'clock; positive angles go clockwise. `TURN / 4` is three
/// o'clock, `TURN / 2` six, and a progress ring at 30% is a sweep of
/// `30 * TURN / 100`.
pub const TURN: i32 = 1 << 16;

/// sin(i / 256 · τ/4) · 2^16, for the first quarter turn inclusive.
///
/// Generated from the real function and pinned by exhaustive tests rather than
/// trusted; see the module documentation.
#[rustfmt::skip]
const SIN_QUARTER: [i32; 257] = [
    0, 402, 804, 1206, 1608, 2010, 2412, 2814,
    3216, 3617, 4019, 4420, 4821, 5222, 5623, 6023,
    6424, 6824, 7224, 7623, 8022, 8421, 8820, 9218,
    9616, 10014, 10411, 10808, 11204, 11600, 11996, 12391,
    12785, 13180, 13573, 13966, 14359, 14751, 15143, 15534,
    15924, 16314, 16703, 17091, 17479, 17867, 18253, 18639,
    19024, 19409, 19792, 20175, 20557, 20939, 21320, 21699,
    22078, 22457, 22834, 23210, 23586, 23961, 24335, 24708,
    25080, 25451, 25821, 26190, 26558, 26925, 27291, 27656,
    28020, 28383, 28745, 29106, 29466, 29824, 30182, 30538,
    30893, 31248, 31600, 31952, 32303, 32652, 33000, 33347,
    33692, 34037, 34380, 34721, 35062, 35401, 35738, 36075,
    36410, 36744, 37076, 37407, 37736, 38064, 38391, 38716,
    39040, 39362, 39683, 40002, 40320, 40636, 40951, 41264,
    41576, 41886, 42194, 42501, 42806, 43110, 43412, 43713,
    44011, 44308, 44604, 44898, 45190, 45480, 45769, 46056,
    46341, 46624, 46906, 47186, 47464, 47741, 48015, 48288,
    48559, 48828, 49095, 49361, 49624, 49886, 50146, 50404,
    50660, 50914, 51166, 51417, 51665, 51911, 52156, 52398,
    52639, 52878, 53114, 53349, 53581, 53812, 54040, 54267,
    54491, 54714, 54934, 55152, 55368, 55582, 55794, 56004,
    56212, 56418, 56621, 56823, 57022, 57219, 57414, 57607,
    57798, 57986, 58172, 58356, 58538, 58718, 58896, 59071,
    59244, 59415, 59583, 59750, 59914, 60075, 60235, 60392,
    60547, 60700, 60851, 60999, 61145, 61288, 61429, 61568,
    61705, 61839, 61971, 62101, 62228, 62353, 62476, 62596,
    62714, 62830, 62943, 63054, 63162, 63268, 63372, 63473,
    63572, 63668, 63763, 63854, 63944, 64031, 64115, 64197,
    64277, 64354, 64429, 64501, 64571, 64639, 64704, 64766,
    64827, 64884, 64940, 64993, 65043, 65091, 65137, 65180,
    65220, 65259, 65294, 65328, 65358, 65387, 65413, 65436,
    65457, 65476, 65492, 65505, 65516, 65525, 65531, 65535,
    65536,
];

/// sin of a binary-turn angle, in Q16.
fn sin_bam(angle: i32) -> i32 {
    let a = angle.rem_euclid(TURN);
    let quarter = TURN / 4;
    let (quadrant, q) = (a / quarter, a % quarter);
    // Fold into the first quarter. The fold reaches q = quarter inclusive, which
    // is the last table entry with nothing to interpolate towards.
    let lookup = |q: i32| -> i32 {
        let idx = (q >> 6) as usize;
        let frac = q & 63;
        if frac == 0 {
            SIN_QUARTER[idx]
        } else {
            SIN_QUARTER[idx] + (SIN_QUARTER[idx + 1] - SIN_QUARTER[idx]) * frac / 64
        }
    };
    match quadrant {
        0 => lookup(q),
        1 => lookup(quarter - q),
        2 => -lookup(q),
        _ => -lookup(quarter - q),
    }
}

/// The unit vector of a clock angle, in Q16 screen coordinates (y down).
///
/// Twelve o'clock is (0, -1), three o'clock (1, 0).
pub fn direction(angle: i32) -> (i32, i32) {
    (sin_bam(angle), -sin_bam(angle + TURN / 4))
}


/// Fractional bits in the fixed-point coordinates the shape code uses.
pub const FRAC_BITS: u32 = 8;

/// One whole pixel in fixed point.
pub const ONE: i32 = 1 << FRAC_BITS;

/// Coordinates are clamped to this before entering fixed point, so a rectangle
/// placed absurdly far off-screen cannot overflow the shift. Any real surface is
/// orders of magnitude inside it.
pub const COORD_LIMIT: i32 = 1 << 22;

/// A whole-pixel coordinate in fixed point, clamped to [`COORD_LIMIT`].
#[inline]
pub fn to_fx(v: i32) -> i32 {
    v.clamp(-COORD_LIMIT, COORD_LIMIT) << FRAC_BITS
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------- the table

    /// The table is data and data can rot, so it is held to the mathematics it
    /// claims to encode: every one of the 65536 angles must satisfy the
    /// Pythagorean identity to about a part in a thousand.
    #[test]
    fn every_angle_satisfies_the_pythagorean_identity() {
        for a in 0..TURN {
            let s = sin_bam(a) as i64;
            let c = sin_bam(a + TURN / 4) as i64;
            let one = (s * s + c * c) >> 16;
            assert!(
                (one - 65536).abs() < 64,
                "angle {a}: sin²+cos² is {one}, not 65536"
            );
        }
    }

    /// The quarters are exact, not approximately right: a progress ring at
    /// exactly 25% must point at exactly three o'clock.
    #[test]
    fn the_cardinal_directions_are_exact() {
        assert_eq!(direction(0), (0, -65536), "twelve o'clock");
        assert_eq!(direction(TURN / 4), (65536, 0), "three o'clock");
        assert_eq!(direction(TURN / 2), (0, 65536), "six o'clock");
        assert_eq!(direction(3 * TURN / 4), (-65536, 0), "nine o'clock");
        assert_eq!(direction(TURN), (0, -65536), "and round again");
        assert_eq!(direction(-TURN / 4), (-65536, 0), "negative wraps too");
    }

    /// Monotone over the first quarter — a table with a transposed pair of
    /// entries would still pass the identity test within tolerance.
    #[test]
    fn sine_rises_monotonically_over_the_first_quarter() {
        let mut previous = -1;
        for a in 0..=TURN / 4 {
            let s = sin_bam(a);
            assert!(s >= previous, "sin fell at angle {a}");
            previous = s;
        }
    }

}
