//! Floating-point constants.
//!
//! An `ApFloat` is a set of IEEE semantics plus a raw bit pattern. Nothing
//! here does floating-point arithmetic: the IR only needs to carry constants
//! from the text into a module and back out again without perturbing a single
//! bit, and a constant folder that needs real arithmetic can be built on top
//! when a pass wants one.
//!
//! The textual forms are LLVM's, which are quirkier than they look. See
//! `to_llvm_text` and `parse_hex_literal` for the exact rules and where they
//! came from.

use crate::apint::ApInt;
use core::fmt;

/// The IEEE (and near-IEEE) formats LLVM IR can name.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FloatSemantics {
    /// IEEE 754 binary16.
    Half,
    /// Brain float: binary32's exponent with 8 bits of mantissa.
    BFloat,
    /// IEEE 754 binary32.
    Single,
    /// IEEE 754 binary64.
    Double,
    /// IEEE 754 binary128.
    Quad,
    /// The x87 80-bit format, with an explicit integer bit.
    X87DoubleExtended,
    /// The PowerPC double-double pair. Carried, never interpreted.
    PpcDoubleDouble,
}

impl FloatSemantics {
    pub fn bit_width(self) -> u32 {
        match self {
            FloatSemantics::Half | FloatSemantics::BFloat => 16,
            FloatSemantics::Single => 32,
            FloatSemantics::Double => 64,
            FloatSemantics::Quad | FloatSemantics::PpcDoubleDouble => 128,
            FloatSemantics::X87DoubleExtended => 80,
        }
    }

    /// The type keyword this format is spelled with in the IR.
    pub fn type_name(self) -> &'static str {
        match self {
            FloatSemantics::Half => "half",
            FloatSemantics::BFloat => "bfloat",
            FloatSemantics::Single => "float",
            FloatSemantics::Double => "double",
            FloatSemantics::Quad => "fp128",
            FloatSemantics::X87DoubleExtended => "x86_fp80",
            FloatSemantics::PpcDoubleDouble => "ppc_fp128",
        }
    }

    /// Bits of stored mantissa, excluding any implicit leading one.
    fn mantissa_bits(self) -> u32 {
        match self {
            FloatSemantics::Half => 10,
            FloatSemantics::BFloat => 7,
            FloatSemantics::Single => 23,
            FloatSemantics::Double => 52,
            FloatSemantics::Quad => 112,
            FloatSemantics::X87DoubleExtended => 63,
            FloatSemantics::PpcDoubleDouble => 52,
        }
    }

    fn exponent_bits(self) -> u32 {
        match self {
            FloatSemantics::Half => 5,
            FloatSemantics::BFloat | FloatSemantics::Single => 8,
            FloatSemantics::Double | FloatSemantics::PpcDoubleDouble => 11,
            FloatSemantics::Quad | FloatSemantics::X87DoubleExtended => 15,
        }
    }
}

/// A floating-point constant: semantics plus the exact bit pattern.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ApFloat {
    semantics: FloatSemantics,
    bits: ApInt,
}

/// Why a textual float constant could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FloatParseError {
    /// The hex prefix letter does not match the type, for example `0xK` on a
    /// `double`.
    WrongHexForm { form: char, ty: &'static str },
    /// The literal has no valid digits.
    Malformed,
    /// The value is not exactly representable in the destination format, which
    /// is what LLVM reports as "floating point constant invalid for type".
    NotRepresentable,
}

impl fmt::Display for FloatParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FloatParseError::WrongHexForm { form, ty } => {
                write!(f, "hexadecimal form 0x{form} is not valid for type {ty}")
            }
            FloatParseError::Malformed => f.write_str("malformed floating-point literal"),
            FloatParseError::NotRepresentable => {
                f.write_str("floating point constant invalid for type")
            }
        }
    }
}

impl std::error::Error for FloatParseError {}

impl ApFloat {
    pub fn from_bits(semantics: FloatSemantics, bits: ApInt) -> Self {
        debug_assert_eq!(bits.bits(), semantics.bit_width());
        ApFloat { semantics, bits }
    }

    pub fn from_raw_u64(semantics: FloatSemantics, raw: u64) -> Self {
        ApFloat {
            semantics,
            bits: ApInt::from_u64(semantics.bit_width(), raw),
        }
    }

    pub fn semantics(&self) -> FloatSemantics {
        self.semantics
    }

    pub fn bits(&self) -> &ApInt {
        &self.bits
    }

    pub fn from_f64(value: f64) -> Self {
        ApFloat {
            semantics: FloatSemantics::Double,
            bits: ApInt::from_u64(64, value.to_bits()),
        }
    }

    pub fn from_f32(value: f32) -> Self {
        ApFloat {
            semantics: FloatSemantics::Single,
            bits: ApInt::from_u64(32, u64::from(value.to_bits())),
        }
    }

    fn sign(&self) -> bool {
        self.bits.bit(self.semantics.bit_width() - 1)
    }

    fn raw_exponent(&self) -> u32 {
        let width = self.semantics.bit_width();
        let ebits = self.semantics.exponent_bits();
        let mut exp = 0u32;
        for i in 0..ebits {
            if self.bits.bit(width - 1 - ebits + i) {
                exp |= 1 << i;
            }
        }
        exp
    }

    fn exponent_all_ones(&self) -> bool {
        self.raw_exponent() == (1u32 << self.semantics.exponent_bits()) - 1
    }

    fn mantissa_is_zero(&self) -> bool {
        (0..self.semantics.mantissa_bits()).all(|i| !self.bits.bit(i))
    }

    pub fn is_nan(&self) -> bool {
        match self.semantics {
            // The double-double pair's classification lives in its high half,
            // and we never interpret it.
            FloatSemantics::PpcDoubleDouble => false,
            FloatSemantics::X87DoubleExtended => {
                self.exponent_all_ones() && !self.mantissa_is_zero_x87()
            }
            _ => self.exponent_all_ones() && !self.mantissa_is_zero(),
        }
    }

    fn mantissa_is_zero_x87(&self) -> bool {
        // Bit 63 is the explicit integer bit; infinity has it set with an
        // otherwise empty mantissa.
        (0..63).all(|i| !self.bits.bit(i))
    }

    pub fn is_infinite(&self) -> bool {
        match self.semantics {
            FloatSemantics::PpcDoubleDouble => false,
            FloatSemantics::X87DoubleExtended => {
                self.exponent_all_ones() && self.mantissa_is_zero_x87()
            }
            _ => self.exponent_all_ones() && self.mantissa_is_zero(),
        }
    }

    pub fn is_finite(&self) -> bool {
        !self.is_nan() && !self.is_infinite()
    }

    pub fn is_zero(&self) -> bool {
        self.raw_exponent() == 0 && self.mantissa_is_zero()
    }

    pub fn is_negative(&self) -> bool {
        self.sign()
    }

    /// The value widened to `f64`, exact for every format that fits.
    pub fn to_f64(&self) -> Option<f64> {
        match self.semantics {
            FloatSemantics::Double => Some(f64::from_bits(self.bits.to_u64()?)),
            FloatSemantics::Single => Some(f64::from(f32::from_bits(self.bits.to_u64()? as u32))),
            FloatSemantics::Half => Some(half_to_f64(self.bits.to_u64()? as u16)),
            FloatSemantics::BFloat => Some(bfloat_to_f64(self.bits.to_u64()? as u16)),
            // 80-bit and 128-bit formats hold values f64 cannot express, and
            // they always print in hexadecimal, so nothing needs this.
            _ => None,
        }
    }

    /// Narrows a `f64` into `semantics`, returning `None` when a bit would be
    /// lost. LLVM performs exactly this check when a literal written for one
    /// type has to land in another.
    pub fn from_f64_exact(semantics: FloatSemantics, value: f64) -> Option<Self> {
        // A NaN narrows by dropping the low mantissa bits, and loses nothing
        // when they are already zero. Comparing bit patterns the way a finite
        // value is compared would refuse every NaN, because narrowing and
        // widening a NaN does not reproduce the payload it started with.
        if value.is_nan()
            && let Some(narrowed) = narrow_nan(semantics, value)
        {
            return Some(narrowed);
        }
        match semantics {
            FloatSemantics::Double => Some(Self::from_f64(value)),
            FloatSemantics::Single => {
                let narrowed = value as f32;
                if f64::from(narrowed).to_bits() == value.to_bits() {
                    Some(Self::from_f32(narrowed))
                } else {
                    None
                }
            }
            FloatSemantics::Half => {
                f64_to_half(value).map(|raw| Self::from_raw_u64(semantics, u64::from(raw)))
            }
            FloatSemantics::BFloat => {
                f64_to_bfloat(value).map(|raw| Self::from_raw_u64(semantics, u64::from(raw)))
            }
            // Widening into the big formats is exact but needs their layouts;
            // no caller wants it yet, so refuse rather than guess.
            _ => None,
        }
    }

    /// Reads one of LLVM's `0x` float literals, with `digits` excluding the
    /// `0x` and the form letter.
    ///
    /// The unprefixed form is a `double` bit pattern whatever the destination
    /// type, so it comes back narrowed and range-checked. The word split of
    /// the 128-bit forms follows upstream's lexer rather than the obvious
    /// reading; `short_quad_literals_keep_upstreams_word_split` pins it.
    pub fn parse_hex_literal(
        form: Option<char>,
        digits: &str,
        target: FloatSemantics,
    ) -> Result<Self, FloatParseError> {
        let require = |want: FloatSemantics| -> Result<(), FloatParseError> {
            if target == want {
                Ok(())
            } else {
                Err(FloatParseError::WrongHexForm {
                    form: form.unwrap_or('x'),
                    ty: target.type_name(),
                })
            }
        };
        match form {
            Some('H') => {
                require(FloatSemantics::Half)?;
                Ok(Self::from_raw_u64(target, parse_hex_u64(digits)?))
            }
            Some('R') => {
                require(FloatSemantics::BFloat)?;
                Ok(Self::from_raw_u64(target, parse_hex_u64(digits)?))
            }
            Some('K') => {
                require(FloatSemantics::X87DoubleExtended)?;
                // First four digits are sign and exponent, the rest is the
                // mantissa including its explicit integer bit.
                let split = digits.len().min(4);
                let high = parse_hex_u64(&digits[..split])?;
                let low = parse_hex_u64(&digits[split..])?;
                Ok(Self::from_bits(target, two_words(80, low, high)))
            }
            Some('L') => {
                require(FloatSemantics::Quad)?;
                let (w0, w1) = split_hex_pair(digits)?;
                Ok(Self::from_bits(target, two_words(128, w0, w1)))
            }
            Some('M') => {
                require(FloatSemantics::PpcDoubleDouble)?;
                let (w0, w1) = split_hex_pair(digits)?;
                Ok(Self::from_bits(target, two_words(128, w0, w1)))
            }
            Some(other) => Err(FloatParseError::WrongHexForm {
                form: other,
                ty: target.type_name(),
            }),
            None => {
                let raw = parse_hex_u64(digits)?;
                let as_double = f64::from_bits(raw);
                if target == FloatSemantics::Double {
                    return Ok(Self::from_f64(as_double));
                }
                Self::from_f64_exact(target, as_double).ok_or(FloatParseError::NotRepresentable)
            }
        }
    }

    /// Reads a decimal or exponential literal into `target`.
    ///
    /// The literal is read as a `double` and then narrowed, which is upstream's
    /// order of operations: the lexer has no type information, so every
    /// non-hexadecimal float starts life as a `double`.
    pub fn parse_decimal(text: &str, target: FloatSemantics) -> Result<Self, FloatParseError> {
        let value: f64 = text.parse().map_err(|_| FloatParseError::Malformed)?;
        if target == FloatSemantics::Double {
            return Ok(Self::from_f64(value));
        }
        Self::from_f64_exact(target, value).ok_or(FloatParseError::NotRepresentable)
    }

    /// The constant as LLVM's assembly writer would print it.
    ///
    /// `float` and `double` get a decimal rendering when a six-digit
    /// exponential form reads back as the same value, and a hexadecimal bit
    /// pattern otherwise. Every other format is always hexadecimal.
    pub fn to_llvm_text(&self) -> String {
        match self.semantics {
            FloatSemantics::Half => format!("0xH{}", self.bits.to_hex_upper_padded()),
            FloatSemantics::BFloat => format!("0xR{}", self.bits.to_hex_upper_padded()),
            FloatSemantics::X87DoubleExtended => {
                let (low, high) = (self.word(0), self.word(1));
                format!("0xK{:04X}{:016X}", high & 0xffff, low)
            }
            FloatSemantics::Quad => {
                format!("0xL{:016X}{:016X}", self.word(0), self.word(1))
            }
            FloatSemantics::PpcDoubleDouble => {
                format!("0xM{:016X}{:016X}", self.word(0), self.word(1))
            }
            FloatSemantics::Single | FloatSemantics::Double => {
                // Widening through the machine's own conversion quietens a
                // signalling NaN, and the printed payload is what says which
                // NaN it was, so a NaN widens by shifting its bits instead.
                if let Some(widened) = widen_nan(self) {
                    return format!("0x{}", ApInt::from_u64(64, widened).to_hex_upper());
                }
                let as_double = self.to_f64().expect("single and double widen to f64");
                if self.is_finite()
                    && let Some(decimal) = exact_decimal_form(as_double)
                {
                    return decimal;
                }
                format!(
                    "0x{}",
                    ApInt::from_u64(64, as_double.to_bits()).to_hex_upper()
                )
            }
        }
    }

    fn word(&self, index: usize) -> u64 {
        self.bits.words().get(index).copied().unwrap_or(0)
    }
}

impl fmt::Debug for ApFloat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.semantics.type_name(), self.to_llvm_text())
    }
}

/// Renders `value` the way upstream's six-significant-digit exponential
/// formatter does, and returns it only when reading it back gives the same
/// value. `None` means the caller must print the bit pattern instead.
fn exact_decimal_form(value: f64) -> Option<String> {
    let text = format_exponential_6(value);
    let reparsed: f64 = text.parse().ok()?;
    // A value comparison, not a bit comparison: upstream compares the two as
    // doubles, so a negative zero that reads back as positive zero still
    // prints in decimal.
    if reparsed == value { Some(text) } else { None }
}

/// `%.6e` in C, which Rust's `{:.6e}` almost but not quite matches: the
/// exponent needs an explicit sign and at least two digits.
fn format_exponential_6(value: f64) -> String {
    let raw = format!("{value:.6e}");
    let (mantissa, exponent) = raw.split_once('e').expect("scientific notation has an e");
    let exponent: i32 = exponent.parse().expect("exponent is an integer");
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("{mantissa}e{sign}{:02}", exponent.abs())
}

fn parse_hex_u64(digits: &str) -> Result<u64, FloatParseError> {
    if digits.is_empty() {
        return Err(FloatParseError::Malformed);
    }
    let mut value = 0u64;
    for c in digits.chars() {
        let digit = c.to_digit(16).ok_or(FloatParseError::Malformed)?;
        value = value.wrapping_mul(16).wrapping_add(u64::from(digit));
    }
    Ok(value)
}

/// Splits a 128-bit hex literal the way upstream does: the first sixteen
/// digits are the low word and the rest are the high word, but only when there
/// are at least sixteen digits to take. Shorter literals put everything in the
/// high word.
fn split_hex_pair(digits: &str) -> Result<(u64, u64), FloatParseError> {
    if digits.len() >= 16 {
        Ok((
            parse_hex_u64(&digits[..16])?,
            parse_hex_u64(&digits[16..]).unwrap_or(0),
        ))
    } else {
        Ok((0, parse_hex_u64(digits)?))
    }
}

fn two_words(bits: u32, low: u64, high: u64) -> ApInt {
    let mut value = ApInt::from_u64(bits, low);
    let shifted = ApInt::from_u64(bits, high).shl(64);
    value = value.or(&shifted);
    value
}

fn half_to_f64(raw: u16) -> f64 {
    let sign = if raw & 0x8000 != 0 { -1.0f64 } else { 1.0 };
    let exponent = i32::from((raw >> 10) & 0x1f);
    let mantissa = f64::from(raw & 0x3ff);
    match exponent {
        0 => sign * mantissa * 2f64.powi(-24),
        0x1f => {
            if mantissa == 0.0 {
                sign * f64::INFINITY
            } else {
                f64::from_bits(f64::NAN.to_bits() | if sign < 0.0 { 1 << 63 } else { 0 })
            }
        }
        _ => sign * (1.0 + mantissa / 1024.0) * 2f64.powi(exponent - 15),
    }
}

fn bfloat_to_f64(raw: u16) -> f64 {
    // bfloat is the top half of a binary32, so widening is a shift.
    f64::from(f32::from_bits(u32::from(raw) << 16))
}

/// Narrows to binary16, or `None` when a bit would be lost.
fn f64_to_half(value: f64) -> Option<u16> {
    narrow_ieee(value, 5, 10).map(|raw| raw as u16)
}

/// Narrows to bfloat16, or `None` when a bit would be lost.
fn f64_to_bfloat(value: f64) -> Option<u16> {
    narrow_ieee(value, 8, 7).map(|raw| raw as u16)
}

/// Exact narrowing of a `f64` into a smaller IEEE format described by its
/// exponent and mantissa widths. Refusing to round is deliberate: this is the
/// path a written-out literal takes, and a literal that cannot be represented
/// is an error rather than something to quietly round.
fn narrow_ieee(value: f64, exponent_bits: u32, mantissa_bits: u32) -> Option<u64> {
    let raw = value.to_bits();
    let sign = raw >> 63;
    let exponent = ((raw >> 52) & 0x7ff) as i32;
    let mantissa = raw & 0x000f_ffff_ffff_ffff;

    let bias = (1i32 << (exponent_bits - 1)) - 1;
    let max_exponent = (1i32 << exponent_bits) - 1;
    let drop = 52 - mantissa_bits;

    if exponent == 0x7ff {
        // Infinities always fit. A NaN keeps its sign and its quiet bit; its
        // payload bits below the target mantissa must be empty to survive.
        if mantissa != 0 && mantissa & ((1u64 << drop) - 1) != 0 {
            return None;
        }
        let narrowed_mantissa = mantissa >> drop;
        return Some(
            (sign << (exponent_bits + mantissa_bits))
                | ((max_exponent as u64) << mantissa_bits)
                | narrowed_mantissa,
        );
    }

    if exponent == 0 && mantissa == 0 {
        return Some(sign << (exponent_bits + mantissa_bits));
    }

    // A f64 subnormal is far below any smaller format's subnormal range.
    if exponent == 0 {
        return None;
    }

    let unbiased = exponent - 1023;
    if unbiased > bias {
        // Overflows the target's finite range.
        return None;
    }

    if unbiased >= 1 - bias {
        // Normal in the target too: the mantissa must fit without rounding.
        if mantissa & ((1u64 << drop) - 1) != 0 {
            return None;
        }
        let narrowed_exponent = (unbiased + bias) as u64;
        return Some(
            (sign << (exponent_bits + mantissa_bits))
                | (narrowed_exponent << mantissa_bits)
                | (mantissa >> drop),
        );
    }

    // Subnormal in the target: shift the implicit one back in and check that
    // nothing falls off the bottom.
    let shift = (1 - bias) - unbiased;
    if shift > mantissa_bits as i32 {
        return None;
    }
    let full = (1u64 << 52) | mantissa;
    let total_drop = drop as i32 + shift;
    if total_drop >= 64 || full & ((1u64 << total_drop) - 1) != 0 {
        return None;
    }
    Some((sign << (exponent_bits + mantissa_bits)) | (full >> total_drop))
}

/// A `double` NaN written into a narrower format, when its payload survives.
///
/// The sign and the exponent carry over whole, and the mantissa keeps its top
/// bits, so the payload is preserved exactly when the bits that fall off the
/// bottom are zero. That is what lets `0x7FF1000000000000` be written as a
/// `float` and `0x7FF0000000000001` not be.
fn narrow_nan(semantics: FloatSemantics, value: f64) -> Option<ApFloat> {
    let mantissa_bits = match semantics {
        FloatSemantics::Single => 23,
        FloatSemantics::Half => 10,
        FloatSemantics::BFloat => 7,
        _ => return None,
    };
    let bits = value.to_bits();
    let dropped = 52 - mantissa_bits;
    if bits & ((1u64 << dropped) - 1) != 0 {
        return None;
    }
    let sign = (bits >> 63) & 1;
    let mantissa = (bits & ((1u64 << 52) - 1)) >> dropped;
    let exponent_bits = semantics.bit_width() - 1 - mantissa_bits;
    let exponent = (1u64 << exponent_bits) - 1;
    let raw = (sign << (semantics.bit_width() - 1)) | (exponent << mantissa_bits) | mantissa;
    Some(ApFloat::from_raw_u64(semantics, raw))
}

/// A `float` NaN as the `double` bit pattern that reads back as it, which is
/// how upstream prints one. `double` needs no widening and everything else
/// prints its own bits.
fn widen_nan(value: &ApFloat) -> Option<u64> {
    if value.semantics() != FloatSemantics::Single || !value.is_nan() {
        return None;
    }
    let raw = value.bits().to_u64()?;
    let sign = (raw >> 31) & 1;
    let mantissa = (raw & ((1 << 23) - 1)) << 29;
    Some((sign << 63) | (0x7ffu64 << 52) | mantissa)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_prints_decimal_when_it_reads_back() {
        assert_eq!(ApFloat::from_f64(1.0).to_llvm_text(), "1.000000e+00");
        assert_eq!(ApFloat::from_f64(0.5).to_llvm_text(), "5.000000e-01");
        assert_eq!(ApFloat::from_f64(-0.0).to_llvm_text(), "-0.000000e+00");
        assert_eq!(ApFloat::from_f64(100.0).to_llvm_text(), "1.000000e+02");
        assert_eq!(ApFloat::from_f64(1e-300).to_llvm_text(), "1.000000e-300");
    }

    #[test]
    fn double_falls_back_to_hex_when_precision_would_be_lost() {
        // Pi needs more than six significant digits, so upstream prints the
        // bit pattern; this is the single most common float in real IR.
        assert_eq!(
            ApFloat::from_f64(std::f64::consts::PI).to_llvm_text(),
            "0x400921FB54442D18"
        );
        assert_eq!(
            ApFloat::from_f64(f64::INFINITY).to_llvm_text(),
            "0x7FF0000000000000"
        );
        assert_eq!(
            ApFloat::from_f64(f64::NEG_INFINITY).to_llvm_text(),
            "0xFFF0000000000000"
        );
        assert_eq!(
            ApFloat::from_f64(f64::NAN).to_llvm_text(),
            "0x7FF8000000000000"
        );
    }

    #[test]
    fn single_prints_through_its_double_widening() {
        assert_eq!(ApFloat::from_f32(1.0).to_llvm_text(), "1.000000e+00");
        assert_eq!(ApFloat::from_f32(0.25).to_llvm_text(), "2.500000e-01");
        // 0.1f32 widens to a double that no six-digit decimal reproduces.
        assert_eq!(ApFloat::from_f32(0.1).to_llvm_text(), "0x3FB99999A0000000");
    }

    #[test]
    fn half_and_bfloat_always_print_hex() {
        let one = ApFloat::from_f64_exact(FloatSemantics::Half, 1.0).unwrap();
        assert_eq!(one.to_llvm_text(), "0xH3C00");
        let three = ApFloat::from_f64_exact(FloatSemantics::Half, 3.0).unwrap();
        assert_eq!(three.to_llvm_text(), "0xH4200");
        let minus_one = ApFloat::from_f64_exact(FloatSemantics::Half, -1.0).unwrap();
        assert_eq!(minus_one.to_llvm_text(), "0xHBC00");
        let bf = ApFloat::from_f64_exact(FloatSemantics::BFloat, 6.0).unwrap();
        assert_eq!(bf.to_llvm_text(), "0xR40C0");
    }

    #[test]
    fn narrowing_refuses_to_round() {
        assert!(ApFloat::from_f64_exact(FloatSemantics::Half, 0.1).is_none());
        assert!(ApFloat::from_f64_exact(FloatSemantics::Half, 1e30).is_none());
        assert!(ApFloat::from_f64_exact(FloatSemantics::Single, 0.1).is_none());
        assert!(ApFloat::from_f64_exact(FloatSemantics::Single, 0.25).is_some());
        // The smallest half subnormal is exact; half of it is not.
        assert!(ApFloat::from_f64_exact(FloatSemantics::Half, 2f64.powi(-24)).is_some());
        assert!(ApFloat::from_f64_exact(FloatSemantics::Half, 2f64.powi(-25)).is_none());
    }

    #[test]
    fn half_widens_back_to_the_same_value() {
        for raw in 0u16..=u16::MAX {
            let value = ApFloat::from_raw_u64(FloatSemantics::Half, u64::from(raw));
            let widened = value.to_f64().unwrap();
            if value.is_nan() {
                assert!(widened.is_nan());
                continue;
            }
            let back = ApFloat::from_f64_exact(FloatSemantics::Half, widened);
            assert_eq!(
                back.map(|v| v.bits().to_u64().unwrap()),
                Some(u64::from(raw)),
                "half 0x{raw:04X} did not survive the round trip"
            );
        }
    }

    #[test]
    fn bfloat_widens_back_to_the_same_value() {
        for raw in 0u16..=u16::MAX {
            let value = ApFloat::from_raw_u64(FloatSemantics::BFloat, u64::from(raw));
            let widened = value.to_f64().unwrap();
            if value.is_nan() {
                assert!(widened.is_nan());
                continue;
            }
            let back = ApFloat::from_f64_exact(FloatSemantics::BFloat, widened);
            assert_eq!(
                back.map(|v| v.bits().to_u64().unwrap()),
                Some(u64::from(raw)),
                "bfloat 0x{raw:04X} did not survive the round trip"
            );
        }
    }

    #[test]
    fn hex_literals_round_trip_through_the_printer() {
        let cases = [
            (FloatSemantics::Half, Some('H'), "4200", "0xH4200"),
            (FloatSemantics::BFloat, Some('R'), "40C0", "0xR40C0"),
            (
                FloatSemantics::X87DoubleExtended,
                Some('K'),
                "4001E000000000000000",
                "0xK4001E000000000000000",
            ),
            (
                FloatSemantics::Quad,
                Some('L'),
                "00000000000000018000000000000000",
                "0xL00000000000000018000000000000000",
            ),
            (
                FloatSemantics::PpcDoubleDouble,
                Some('M'),
                "80000000000000000000000000000000",
                "0xM80000000000000000000000000000000",
            ),
        ];
        for (semantics, form, digits, printed) in cases {
            let value = ApFloat::parse_hex_literal(form, digits, semantics).unwrap();
            assert_eq!(value.to_llvm_text(), printed);
        }
    }

    #[test]
    fn short_quad_literals_keep_upstreams_word_split() {
        // Upstream's Assembler/short-hexpair.ll pins this: `0xL01` reads as
        // the same value as the zero-padded spelling and prints padded. The
        // digits land in the second word, and the second word is what gets
        // printed second, so the two agree.
        let short = ApFloat::parse_hex_literal(Some('L'), "01", FloatSemantics::Quad).unwrap();
        let padded = ApFloat::parse_hex_literal(
            Some('L'),
            "00000000000000000000000000000001",
            FloatSemantics::Quad,
        )
        .unwrap();
        assert_eq!(short, padded);
        assert_eq!(short.to_llvm_text(), "0xL00000000000000000000000000000001");
    }

    #[test]
    fn quad_prints_its_low_word_first() {
        // 1.0 in binary128 is sign 0, exponent 0x3fff, empty mantissa, and
        // upstream prints it as the mantissa word followed by the word
        // carrying the exponent. Nineteen files in upstream's test tree spell
        // it exactly this way.
        let one = ApFloat::parse_hex_literal(
            Some('L'),
            "00000000000000003FFF000000000000",
            FloatSemantics::Quad,
        )
        .unwrap();
        assert_eq!(one.to_llvm_text(), "0xL00000000000000003FFF000000000000");
        assert!(one.is_finite());
        assert!(!one.is_zero());
        assert!(!one.is_negative());
    }

    #[test]
    fn unprefixed_hex_is_a_double_pattern() {
        let d =
            ApFloat::parse_hex_literal(None, "3FF0000000000000", FloatSemantics::Double).unwrap();
        assert_eq!(d.to_f64(), Some(1.0));
        // Legal for float only because the low bits are clear.
        let f =
            ApFloat::parse_hex_literal(None, "3FF0000000000000", FloatSemantics::Single).unwrap();
        assert_eq!(f.to_f64(), Some(1.0));
        assert_eq!(
            ApFloat::parse_hex_literal(None, "3FF0000000000001", FloatSemantics::Single),
            Err(FloatParseError::NotRepresentable)
        );
        assert_eq!(
            ApFloat::parse_hex_literal(None, "7FF0000000000000", FloatSemantics::Single)
                .unwrap()
                .to_llvm_text(),
            "0x7FF0000000000000"
        );
    }

    #[test]
    fn wrong_hex_form_for_the_type_is_rejected() {
        assert_eq!(
            ApFloat::parse_hex_literal(Some('K'), "0000", FloatSemantics::Double),
            Err(FloatParseError::WrongHexForm {
                form: 'K',
                ty: "double"
            })
        );
        assert_eq!(
            ApFloat::parse_hex_literal(Some('H'), "3C00", FloatSemantics::BFloat),
            Err(FloatParseError::WrongHexForm {
                form: 'H',
                ty: "bfloat"
            })
        );
    }

    #[test]
    fn decimal_literals_narrow_or_fail() {
        assert_eq!(
            ApFloat::parse_decimal("1.0", FloatSemantics::Double)
                .unwrap()
                .to_f64(),
            Some(1.0)
        );
        assert_eq!(
            ApFloat::parse_decimal("1.25", FloatSemantics::Single)
                .unwrap()
                .to_f64(),
            Some(1.25)
        );
        assert_eq!(
            ApFloat::parse_decimal("1.3", FloatSemantics::Single),
            Err(FloatParseError::NotRepresentable)
        );
        assert_eq!(
            ApFloat::parse_decimal("nonsense", FloatSemantics::Double),
            Err(FloatParseError::Malformed)
        );
    }

    #[test]
    fn classification() {
        assert!(ApFloat::from_f64(f64::NAN).is_nan());
        assert!(!ApFloat::from_f64(f64::NAN).is_infinite());
        assert!(ApFloat::from_f64(f64::INFINITY).is_infinite());
        assert!(ApFloat::from_f64(0.0).is_zero());
        assert!(ApFloat::from_f64(-0.0).is_zero());
        assert!(ApFloat::from_f64(-0.0).is_negative());
        assert!(!ApFloat::from_f64(0.0).is_negative());
        assert!(ApFloat::from_f64(1.0).is_finite());
    }
}
