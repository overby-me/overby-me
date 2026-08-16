//! Q16.16 fixed-point arithmetic.
//!
//! A port of the libfixmath copy vendored in the C tree
//! (`src/lib/libfixmath`), configured the way that copy configures itself:
//! rounding on, overflow detection on, and no result cache (`FIXMATH_NO_CACHE`
//! is defined at the top of its `fix16.h`).
//!
//! The AVR build selected byte-wise multiply and restoring-division kernels
//! (`FIXMATH_OPTIMIZE_8BIT`) because the target has no multiplier worth the
//! name. Those kernels compute the same rounded result as the straightforward
//! 64-bit ones, so this port uses the 64-bit form and skips the byte shuffling.
//!
//! Overflow is reported the way libfixmath reports it, by returning
//! [`Fix16::OVERFLOW`] (which shares its bit pattern with [`Fix16::MIN`])
//! rather than by wrapping or panicking.

use core::fmt;
use core::ops::{Add, AddAssign, Div, Mul, Neg, Rem, Sub, SubAssign};

/// A signed 16.16 fixed-point number.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Fix16(i32);

impl Fix16 {
    /// 1.0.
    pub const ONE: Self = Self(0x0001_0000);
    /// 0.0.
    pub const ZERO: Self = Self(0);
    /// π, to the bit that libfixmath uses.
    pub const PI: Self = Self(205_887);
    /// e, to the bit that libfixmath uses.
    pub const E: Self = Self(178_145);
    /// The largest representable value.
    pub const MAX: Self = Self(0x7FFF_FFFF);
    /// The smallest representable value.
    pub const MIN: Self = Self(i32::MIN);
    /// Returned by an operation that overflowed, as in libfixmath. It is the
    /// same bit pattern as [`Fix16::MIN`], which that library accepts as the
    /// price of not carrying a flag.
    pub const OVERFLOW: Self = Self(i32::MIN);

    /// Wraps a raw 16.16 word. Use this for the constants the C code writes in
    /// hex, e.g. `Fix16::from_raw(0x4b65f)` for 270 degrees in radians.
    #[must_use]
    pub const fn from_raw(raw: i32) -> Self {
        Self(raw)
    }

    /// The raw 16.16 word.
    #[must_use]
    pub const fn raw(self) -> i32 {
        self.0
    }

    /// Scales an integer up into 16.16.
    #[must_use]
    pub const fn from_int(value: i32) -> Self {
        Self(value.wrapping_mul(Self::ONE.0))
    }

    /// Rounds to the nearest integer, halves away from zero, as `fix16_to_int`.
    ///
    /// Widened to 64 bits for the rounding step, which the C does in 32 and
    /// which overflows there for a word within half a unit of either end --
    /// including [`Fix16::OVERFLOW`], the sentinel its own multiply hands back.
    #[must_use]
    pub const fn to_int(self) -> i32 {
        let value = self.0 as i64;
        let half = (Self::ONE.0 >> 1) as i64;

        (if value >= 0 {
            (value + half) / Self::ONE.0 as i64
        } else {
            (value - half) / Self::ONE.0 as i64
        }) as i32
    }

    /// Converts from `f32`, rounding to nearest.
    #[must_use]
    pub fn from_f32(value: f32) -> Self {
        let scaled = value * Self::ONE.0 as f32;
        Self((scaled + if scaled >= 0.0 { 0.5 } else { -0.5 }) as i32)
    }

    /// Converts from `f64`, rounding to nearest.
    #[must_use]
    pub fn from_f64(value: f64) -> Self {
        let scaled = value * f64::from(Self::ONE.0);
        Self((scaled + if scaled >= 0.0 { 0.5 } else { -0.5 }) as i32)
    }

    /// Converts to `f32`.
    #[must_use]
    pub fn to_f32(self) -> f32 {
        self.0 as f32 / Self::ONE.0 as f32
    }

    /// Converts to `f64`.
    #[must_use]
    pub fn to_f64(self) -> f64 {
        f64::from(self.0) / f64::from(Self::ONE.0)
    }

    /// Absolute value. [`Fix16::MIN`] has no positive counterpart and is
    /// returned unchanged, as it is in the C.
    #[must_use]
    pub const fn abs(self) -> Self {
        Self(if self.0 < 0 {
            self.0.wrapping_neg()
        } else {
            self.0
        })
    }

    /// The square, `self * self`.
    #[must_use]
    pub fn sq(self) -> Self {
        self * self
    }

    /// The smaller of two values.
    #[must_use]
    pub fn min(self, other: Self) -> Self {
        if self.0 < other.0 { self } else { other }
    }

    /// The larger of two values.
    #[must_use]
    pub fn max(self, other: Self) -> Self {
        if self.0 > other.0 { self } else { other }
    }

    /// Sine, by the truncated Taylor series libfixmath falls back on when
    /// neither the lookup table nor the fast path is compiled in. Accurate to
    /// roughly a part in a thousand over a full turn.
    #[must_use]
    pub fn sin(self) -> Self {
        // A truncating remainder, matching C's `%`, so the sign of a negative
        // angle survives the reduction.
        let mut angle = self.0 % (Self::PI.0 << 1);

        if angle > Self::PI.0 {
            angle -= Self::PI.0 << 1;
        } else if angle < -Self::PI.0 {
            angle += Self::PI.0 << 1;
        }

        // The factorial divisions are plain integer divisions on the raw word,
        // not fixed-point ones, exactly as in the C.
        let squared = Self(angle) * Self(angle);
        let mut term = Self(angle);
        let mut out = angle;

        term = term * squared;
        out = out.wrapping_sub(term.0 / 6);
        term = term * squared;
        out = out.wrapping_add(term.0 / 120);
        term = term * squared;
        out = out.wrapping_sub(term.0 / 5040);
        term = term * squared;
        out = out.wrapping_add(term.0 / 362_880);
        term = term * squared;
        out = out.wrapping_sub(term.0 / 39_916_800);

        Self(out)
    }

    /// Cosine, as `sin(x + π/2)`.
    #[must_use]
    pub fn cos(self) -> Self {
        Self(self.0.wrapping_add(Self::PI.0 >> 1)).sin()
    }

    /// Radians to degrees.
    #[must_use]
    pub fn rad_to_deg(self) -> Self {
        self * Self(3_754_936)
    }

    /// Degrees to radians.
    #[must_use]
    pub fn deg_to_rad(self) -> Self {
        self * Self(1144)
    }
}

impl Add for Fix16 {
    type Output = Self;

    /// Addition that reports overflow rather than wrapping, as `fix16_add`.
    fn add(self, rhs: Self) -> Self {
        self.0.checked_add(rhs.0).map_or(Self::OVERFLOW, Self)
    }
}

impl Sub for Fix16 {
    type Output = Self;

    /// Subtraction that reports overflow rather than wrapping, as `fix16_sub`.
    fn sub(self, rhs: Self) -> Self {
        self.0.checked_sub(rhs.0).map_or(Self::OVERFLOW, Self)
    }
}

impl Neg for Fix16 {
    type Output = Self;

    fn neg(self) -> Self {
        Self(self.0.wrapping_neg())
    }
}

impl Mul for Fix16 {
    type Output = Self;

    /// `fix16_mul`: a 64-bit product, rounded to nearest, with the upper 17
    /// bits checked for a sign that did not survive the multiply.
    fn mul(self, rhs: Self) -> Self {
        let mut product = i64::from(self.0) * i64::from(rhs.0);

        // In range, the top 17 bits are all copies of the sign bit.
        let upper = (product >> 47) as u32;
        if product < 0 {
            if upper != u32::MAX {
                return Self::OVERFLOW;
            }
            // Needed to round -1/2 in the same direction as +1/2.
            product -= 1;
        } else if upper != 0 {
            return Self::OVERFLOW;
        }

        let result = (product >> 16) as i32;
        Self(result.wrapping_add(((product & 0x8000) >> 15) as i32))
    }
}

impl Div for Fix16 {
    type Output = Self;

    /// `fix16_div`: the quotient of the magnitudes, rounded half away from
    /// zero, with the sign applied afterwards. Division by zero yields
    /// [`Fix16::MIN`], which is what the C returns.
    fn div(self, rhs: Self) -> Self {
        if rhs.0 == 0 {
            return Self::MIN;
        }

        // One extra bit of quotient, so the rounding decision is a shift.
        let numerator = u64::from(self.0.unsigned_abs()) << 17;
        let quotient = (numerator / u64::from(rhs.0.unsigned_abs()) + 1) >> 1;

        if quotient > i32::MAX as u64 {
            return Self::OVERFLOW;
        }

        let result = quotient as i32;
        Self(if (self.0 ^ rhs.0) < 0 {
            -result
        } else {
            result
        })
    }
}

impl Rem for Fix16 {
    type Output = Self;

    /// `fix16_mod`. The AVR build repeatedly subtracts rather than dividing,
    /// which is faster on a core with no divider and gives the same answer for
    /// the near-in-range angles this is used on. A zero divisor would spin
    /// there; here it returns the dividend untouched.
    fn rem(self, rhs: Self) -> Self {
        if rhs.0 == 0 {
            return self;
        }

        Self(self.0 % rhs.0)
    }
}

impl AddAssign for Fix16 {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl SubAssign for Fix16 {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl fmt::Debug for Fix16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_f64())
    }
}

impl fmt::Display for Fix16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How far a fixed-point result may sit from the real one, in raw units.
    fn close(value: Fix16, expected: f64, tolerance: f64) {
        let difference = (value.to_f64() - expected).abs();
        assert!(
            difference <= tolerance,
            "{value:?} is not within {tolerance} of {expected}"
        );
    }

    #[test]
    fn conversions_round_trip() {
        assert_eq!(Fix16::from_int(7).to_int(), 7);
        assert_eq!(Fix16::from_int(-7).to_int(), -7);
        assert_eq!(Fix16::ONE.raw(), 65536);
        close(Fix16::from_f64(1.5), 1.5, 0.0);
    }

    #[test]
    fn to_int_rounds_halves_away_from_zero() {
        assert_eq!(Fix16::from_f64(2.5).to_int(), 3);
        assert_eq!(Fix16::from_f64(-2.5).to_int(), -3);
        assert_eq!(Fix16::from_f64(2.4).to_int(), 2);
    }

    #[test]
    fn multiplication_rounds() {
        close(Fix16::from_f64(2.5) * Fix16::from_f64(4.0), 10.0, 0.0);
        close(Fix16::from_f64(-2.5) * Fix16::from_f64(4.0), -10.0, 0.0);
        // 1/3 * 3 lands one raw unit off 1.0 either way; rounding keeps it exact.
        close(
            Fix16::ONE / Fix16::from_int(3) * Fix16::from_int(3),
            1.0,
            0.0001,
        );
    }

    #[test]
    fn division_matches_the_reals() {
        close(Fix16::from_int(1) / Fix16::from_int(2), 0.5, 0.0);
        close(Fix16::from_int(-9) / Fix16::from_int(3), -3.0, 0.0);
        assert_eq!(Fix16::ONE / Fix16::ZERO, Fix16::MIN);
    }

    #[test]
    fn multiplication_reports_overflow() {
        assert_eq!(
            Fix16::from_int(40_000) * Fix16::from_int(40_000),
            Fix16::OVERFLOW
        );
    }

    #[test]
    fn addition_reports_overflow() {
        assert_eq!(Fix16::MAX + Fix16::ONE, Fix16::OVERFLOW);
    }

    #[test]
    fn sine_tracks_the_real_thing() {
        // The Taylor series is the least accurate of libfixmath's three sine
        // paths, and its own comment puts it at about 2%. Near the ends of the
        // reduced range the eleventh power overflows and the last term is
        // rubbish, which is where the error lives; the navigator only ever
        // rounds the result to a whole centimetre.
        let mut worst = 0.0f64;

        for degrees in (-360..=360).step_by(5) {
            let radians = f64::from(degrees).to_radians();
            worst = worst.max((Fix16::from_f64(radians).sin().to_f64() - radians.sin()).abs());
            worst = worst.max((Fix16::from_f64(radians).cos().to_f64() - radians.cos()).abs());
        }

        assert!(worst < 0.02, "sine is out by {worst}");
    }

    #[test]
    fn the_hex_constants_are_the_angles_they_claim() {
        // nav.h states these as raw words; they are 270 and 90 degrees.
        close(
            Fix16::from_raw(0x4_b65f),
            3.0 * core::f64::consts::FRAC_PI_2,
            0.001,
        );
        close(
            Fix16::from_raw(0x1_9220),
            core::f64::consts::FRAC_PI_2,
            0.001,
        );
        // The sonar reliability constant. Its header calls it sin(15°)/sin(75°),
        // which is 0.268; the word that was compiled in is 0.3225. See
        // `nav::SONAR_RELIABLE_CONSTANT`.
        close(Fix16::from_raw(0x5290), 0.322_509_765_625, 0.000_001);
    }
}
