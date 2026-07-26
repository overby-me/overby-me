//! Arbitrary-precision integers.
//!
//! Every integer constant in the IR is an `ApInt`: a width in bits plus a
//! two's-complement value stored in little-endian 64-bit words. Widths from
//! `i1` to `i8388608` are legal in LLVM IR, so a fixed 128-bit value type is
//! not enough even though almost all real IR stays inside one or two words.
//!
//! There is exactly one implementation of each operation. Narrow values live
//! in an inline two-word array instead of on the heap, but they run the same
//! word-slice code as wide ones, so the common path and the rare path cannot
//! drift apart.

use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};

const WORD_BITS: u32 = 64;
const INLINE_WORDS: usize = 2;

/// A two's-complement integer of a fixed bit width.
///
/// The value is always canonical: every bit above `bits` is zero, so equality
/// and hashing can look at the raw words.
#[derive(Clone)]
pub struct ApInt {
    bits: u32,
    repr: Repr,
}

#[derive(Clone)]
enum Repr {
    Inline([u64; INLINE_WORDS]),
    Heap(Box<[u64]>),
}

/// Why a string could not be read as an integer of a given width.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseIntError {
    /// The string was empty, or held something that is not a digit in the
    /// requested radix.
    InvalidDigit(char),
    /// The string held no digits at all.
    Empty,
    /// The value does not fit the requested width, as either a signed or an
    /// unsigned quantity.
    OutOfRange,
}

impl fmt::Display for ParseIntError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseIntError::InvalidDigit(c) => write!(f, "invalid digit '{c}'"),
            ParseIntError::Empty => f.write_str("no digits"),
            ParseIntError::OutOfRange => f.write_str("value does not fit the integer width"),
        }
    }
}

impl std::error::Error for ParseIntError {}

const fn words_for(bits: u32) -> usize {
    if bits == 0 {
        1
    } else {
        bits.div_ceil(WORD_BITS) as usize
    }
}

/// Mask of the live bits in the most significant word.
const fn top_word_mask(bits: u32) -> u64 {
    let rem = bits % WORD_BITS;
    if rem == 0 {
        u64::MAX
    } else {
        (1u64 << rem) - 1
    }
}

impl ApInt {
    /// Zero of the given width. Width 0 is not a legal IR type; it is accepted
    /// here and behaves as a one-word zero so callers do not have to special
    /// case it before reporting their own error.
    pub fn new(bits: u32) -> Self {
        let n = words_for(bits);
        let repr = if n <= INLINE_WORDS {
            Repr::Inline([0; INLINE_WORDS])
        } else {
            Repr::Heap(vec![0; n].into_boxed_slice())
        };
        ApInt { bits, repr }
    }

    pub fn from_u64(bits: u32, value: u64) -> Self {
        let mut v = Self::new(bits);
        v.words_mut()[0] = value;
        v.canonicalize();
        v
    }

    pub fn from_u128(bits: u32, value: u128) -> Self {
        let mut v = Self::new(bits);
        {
            let w = v.words_mut();
            w[0] = value as u64;
            if w.len() > 1 {
                w[1] = (value >> 64) as u64;
            }
        }
        v.canonicalize();
        v
    }

    pub fn from_i64(bits: u32, value: i64) -> Self {
        Self::from_i128(bits, i128::from(value))
    }

    /// Sign-extends `value` across the whole width before truncating, so
    /// `from_i128(8, -1)` is `0xff` and not `0x...ff` clipped by accident.
    pub fn from_i128(bits: u32, value: i128) -> Self {
        let mut v = Self::from_u128(bits.max(128), value as u128);
        if value < 0 {
            let w = v.words_mut();
            for word in w.iter_mut().skip(2) {
                *word = u64::MAX;
            }
        }
        v.canonicalize();
        v.trunc_or_self(bits)
    }

    pub fn one(bits: u32) -> Self {
        Self::from_u64(bits, 1)
    }

    pub fn all_ones(bits: u32) -> Self {
        let mut v = Self::new(bits);
        for word in v.words_mut() {
            *word = u64::MAX;
        }
        v.canonicalize();
        v
    }

    /// The most negative signed value of this width: `1 << (bits - 1)`.
    pub fn signed_min(bits: u32) -> Self {
        let mut v = Self::new(bits);
        if bits > 0 {
            v.set_bit(bits - 1, true);
        }
        v
    }

    /// The most positive signed value of this width.
    pub fn signed_max(bits: u32) -> Self {
        let mut v = Self::all_ones(bits);
        if bits > 0 {
            v.set_bit(bits - 1, false);
        }
        v
    }

    pub fn bits(&self) -> u32 {
        self.bits
    }

    pub fn words(&self) -> &[u64] {
        let n = words_for(self.bits);
        match &self.repr {
            Repr::Inline(w) => &w[..n],
            Repr::Heap(w) => w,
        }
    }

    fn words_mut(&mut self) -> &mut [u64] {
        let n = words_for(self.bits);
        match &mut self.repr {
            Repr::Inline(w) => &mut w[..n],
            Repr::Heap(w) => w,
        }
    }

    /// Clears the dead bits above `bits` in the top word.
    fn canonicalize(&mut self) {
        let bits = self.bits;
        let mask = top_word_mask(bits);
        let w = self.words_mut();
        if let Some(top) = w.last_mut() {
            *top &= mask;
        }
    }

    fn trunc_or_self(self, bits: u32) -> Self {
        if bits >= self.bits {
            self
        } else {
            self.trunc(bits)
        }
    }

    pub fn is_zero(&self) -> bool {
        self.words().iter().all(|w| *w == 0)
    }

    pub fn is_one(&self) -> bool {
        self.words()[0] == 1 && self.words()[1..].iter().all(|w| *w == 0)
    }

    pub fn is_all_ones(&self) -> bool {
        let n = self.words().len();
        self.words()[..n - 1].iter().all(|w| *w == u64::MAX)
            && self.words()[n - 1] == top_word_mask(self.bits)
    }

    /// True when the sign bit is set, which only means "negative" if the value
    /// is being read as signed.
    pub fn is_negative(&self) -> bool {
        self.bits > 0 && self.bit(self.bits - 1)
    }

    pub fn bit(&self, index: u32) -> bool {
        if index >= self.bits {
            return false;
        }
        let w = (index / WORD_BITS) as usize;
        let b = index % WORD_BITS;
        (self.words()[w] >> b) & 1 == 1
    }

    pub fn set_bit(&mut self, index: u32, value: bool) {
        if index >= self.bits {
            return;
        }
        let w = (index / WORD_BITS) as usize;
        let b = index % WORD_BITS;
        let words = self.words_mut();
        if value {
            words[w] |= 1u64 << b;
        } else {
            words[w] &= !(1u64 << b);
        }
    }

    /// The value as `u64`, or `None` when it needs more than 64 bits.
    pub fn to_u64(&self) -> Option<u64> {
        if self.words()[1..].iter().any(|w| *w != 0) {
            return None;
        }
        Some(self.words()[0])
    }

    /// The value as `u64`, discarding anything above bit 63.
    pub fn to_u64_truncating(&self) -> u64 {
        self.words()[0]
    }

    pub fn to_u128(&self) -> Option<u128> {
        if self.words().len() > 2 && self.words()[2..].iter().any(|w| *w != 0) {
            return None;
        }
        Some(self.to_u128_truncating())
    }

    pub fn to_u128_truncating(&self) -> u128 {
        let w = self.words();
        let lo = u128::from(w[0]);
        let hi = if w.len() > 1 { u128::from(w[1]) } else { 0 };
        lo | (hi << 64)
    }

    /// The value read as signed, or `None` when it does not fit `i128`.
    pub fn to_i128(&self) -> Option<i128> {
        let extended = self.sext(self.bits.max(128));
        if extended.bits > 128 && !extended.is_sign_extended_beyond(128) {
            return None;
        }
        Some(extended.to_u128_truncating() as i128)
    }

    /// True when every bit at or above `from` matches bit `from - 1`.
    fn is_sign_extended_beyond(&self, from: u32) -> bool {
        let sign = self.bit(from - 1);
        (from..self.bits).all(|i| self.bit(i) == sign)
    }

    pub fn zext(&self, bits: u32) -> Self {
        debug_assert!(bits >= self.bits, "zext must widen");
        let mut out = Self::new(bits);
        let src = self.words();
        let dst = out.words_mut();
        let n = src.len().min(dst.len());
        dst[..n].copy_from_slice(&src[..n]);
        out.canonicalize();
        out
    }

    pub fn sext(&self, bits: u32) -> Self {
        debug_assert!(bits >= self.bits, "sext must widen");
        let mut out = self.zext(bits);
        if self.is_negative() {
            for i in self.bits..bits {
                out.set_bit(i, true);
            }
        }
        out
    }

    pub fn trunc(&self, bits: u32) -> Self {
        debug_assert!(bits <= self.bits, "trunc must narrow");
        let mut out = Self::new(bits);
        let src = self.words();
        let dst = out.words_mut();
        let n = src.len().min(dst.len());
        dst[..n].copy_from_slice(&src[..n]);
        out.canonicalize();
        out
    }

    /// Widen with zeroes or narrow, whichever the target width calls for.
    pub fn zext_or_trunc(&self, bits: u32) -> Self {
        match bits.cmp(&self.bits) {
            Ordering::Greater => self.zext(bits),
            Ordering::Less => self.trunc(bits),
            Ordering::Equal => self.clone(),
        }
    }

    /// Widen with the sign bit or narrow, whichever the target width calls for.
    pub fn sext_or_trunc(&self, bits: u32) -> Self {
        match bits.cmp(&self.bits) {
            Ordering::Greater => self.sext(bits),
            Ordering::Less => self.trunc(bits),
            Ordering::Equal => self.clone(),
        }
    }

    pub fn not(&self) -> Self {
        let mut out = self.clone();
        for w in out.words_mut() {
            *w = !*w;
        }
        out.canonicalize();
        out
    }

    fn bitwise(&self, other: &Self, op: impl Fn(u64, u64) -> u64) -> Self {
        debug_assert_eq!(self.bits, other.bits, "width mismatch");
        let mut out = self.clone();
        let rhs = other.words().to_vec();
        for (w, r) in out.words_mut().iter_mut().zip(rhs) {
            *w = op(*w, r);
        }
        out.canonicalize();
        out
    }

    pub fn and(&self, other: &Self) -> Self {
        self.bitwise(other, |a, b| a & b)
    }

    pub fn or(&self, other: &Self) -> Self {
        self.bitwise(other, |a, b| a | b)
    }

    pub fn xor(&self, other: &Self) -> Self {
        self.bitwise(other, |a, b| a ^ b)
    }

    /// Shift left. A shift amount at or above the width produces zero, which
    /// is `poison` in the IR but a defined value here; the folder decides.
    pub fn shl(&self, amount: u32) -> Self {
        if amount >= self.bits {
            return Self::new(self.bits);
        }
        let mut out = Self::new(self.bits);
        let word_shift = (amount / WORD_BITS) as usize;
        let bit_shift = amount % WORD_BITS;
        let src = self.words();
        {
            let dst = out.words_mut();
            for i in (word_shift..dst.len()).rev() {
                let lo = src[i - word_shift];
                let carried = if bit_shift == 0 {
                    lo
                } else {
                    let below = if i - word_shift == 0 {
                        0
                    } else {
                        src[i - word_shift - 1] >> (WORD_BITS - bit_shift)
                    };
                    (lo << bit_shift) | below
                };
                dst[i] = carried;
            }
        }
        out.canonicalize();
        out
    }

    /// Logical shift right, filling with zeroes.
    pub fn lshr(&self, amount: u32) -> Self {
        if amount >= self.bits {
            return Self::new(self.bits);
        }
        let mut out = Self::new(self.bits);
        let word_shift = (amount / WORD_BITS) as usize;
        let bit_shift = amount % WORD_BITS;
        let src = self.words();
        {
            let dst = out.words_mut();
            for i in 0..dst.len() - word_shift {
                let hi = src[i + word_shift];
                dst[i] = if bit_shift == 0 {
                    hi
                } else {
                    let above = if i + word_shift + 1 < src.len() {
                        src[i + word_shift + 1] << (WORD_BITS - bit_shift)
                    } else {
                        0
                    };
                    (hi >> bit_shift) | above
                };
            }
        }
        out.canonicalize();
        out
    }

    /// Arithmetic shift right, filling with the sign bit.
    pub fn ashr(&self, amount: u32) -> Self {
        let negative = self.is_negative();
        if amount >= self.bits {
            return if negative {
                Self::all_ones(self.bits)
            } else {
                Self::new(self.bits)
            };
        }
        let mut out = self.lshr(amount);
        if negative {
            for i in (self.bits - amount)..self.bits {
                out.set_bit(i, true);
            }
        }
        out
    }

    /// Two's-complement negation.
    pub fn negate(&self) -> Self {
        self.not().wrapping_add(&Self::one(self.bits))
    }

    /// Absolute value read as signed. The most negative value negates to
    /// itself, exactly as it does on a machine.
    pub fn abs_signed(&self) -> Self {
        if self.is_negative() {
            self.negate()
        } else {
            self.clone()
        }
    }

    pub fn wrapping_add(&self, other: &Self) -> Self {
        self.carrying_add(other).0
    }

    /// Sum plus the carry out of the top bit, which is unsigned overflow.
    pub fn carrying_add(&self, other: &Self) -> (Self, bool) {
        debug_assert_eq!(self.bits, other.bits, "width mismatch");
        let mut out = Self::new(self.bits);
        let a = self.words();
        let b = other.words();
        let mut carry = 0u64;
        {
            let dst = out.words_mut();
            for i in 0..dst.len() {
                let (s1, c1) = a[i].overflowing_add(b[i]);
                let (s2, c2) = s1.overflowing_add(carry);
                dst[i] = s2;
                carry = u64::from(c1 || c2);
            }
        }
        // The carry out of a partially used top word comes from the bit above
        // the width, not from the word boundary.
        let overflow = if self.bits.is_multiple_of(WORD_BITS) {
            carry == 1
        } else {
            out.words()
                .last()
                .is_some_and(|w| *w & !top_word_mask(self.bits) != 0)
        };
        out.canonicalize();
        (out, overflow)
    }

    pub fn wrapping_sub(&self, other: &Self) -> Self {
        self.borrowing_sub(other).0
    }

    /// Difference plus the borrow out, which is unsigned underflow.
    pub fn borrowing_sub(&self, other: &Self) -> (Self, bool) {
        debug_assert_eq!(self.bits, other.bits, "width mismatch");
        let borrow = self.cmp_unsigned(other) == Ordering::Less;
        (self.wrapping_add(&other.negate()), borrow)
    }

    pub fn wrapping_mul(&self, other: &Self) -> Self {
        debug_assert_eq!(self.bits, other.bits, "width mismatch");
        let a = self.words();
        let b = other.words();
        let n = a.len();
        let mut acc = vec![0u64; n];
        for (i, &ai) in a.iter().enumerate() {
            if ai == 0 {
                continue;
            }
            let mut carry = 0u128;
            for j in 0..(n - i) {
                let product = u128::from(ai) * u128::from(b[j]) + u128::from(acc[i + j]) + carry;
                acc[i + j] = product as u64;
                carry = product >> 64;
            }
        }
        let mut out = Self::new(self.bits);
        out.words_mut().copy_from_slice(&acc);
        out.canonicalize();
        out
    }

    /// True when the unsigned product does not fit the width.
    pub fn unsigned_mul_overflows(&self, other: &Self) -> bool {
        let wide = self.bits * 2;
        let product = self.zext(wide).wrapping_mul(&other.zext(wide));
        product.lshr(self.bits) != Self::new(wide)
    }

    /// True when the signed product does not fit the width.
    pub fn signed_mul_overflows(&self, other: &Self) -> bool {
        let wide = self.bits * 2;
        let product = self.sext(wide).wrapping_mul(&other.sext(wide));
        product.trunc(self.bits).sext(wide) != product
    }

    /// True when the signed sum does not fit the width: both operands share a
    /// sign and the result does not.
    pub fn signed_add_overflows(&self, other: &Self) -> bool {
        let sum = self.wrapping_add(other);
        self.is_negative() == other.is_negative() && sum.is_negative() != self.is_negative()
    }

    /// True when the signed difference does not fit the width.
    pub fn signed_sub_overflows(&self, other: &Self) -> bool {
        let diff = self.wrapping_sub(other);
        self.is_negative() != other.is_negative() && diff.is_negative() != self.is_negative()
    }

    /// Unsigned quotient and remainder. `None` when the divisor is zero.
    pub fn udivrem(&self, other: &Self) -> Option<(Self, Self)> {
        debug_assert_eq!(self.bits, other.bits, "width mismatch");
        if other.is_zero() {
            return None;
        }
        if self.cmp_unsigned(other) == Ordering::Less {
            return Some((Self::new(self.bits), self.clone()));
        }
        // Shift-subtract long division, one bit at a time. Divisions of IR
        // constants are rare enough that a schoolbook loop is the right call
        // over Knuth algorithm D and its corner cases.
        let mut quotient = Self::new(self.bits);
        let mut remainder = Self::new(self.bits);
        for i in (0..self.bits).rev() {
            remainder = remainder.shl(1);
            remainder.set_bit(0, self.bit(i));
            if remainder.cmp_unsigned(other) != Ordering::Less {
                remainder = remainder.wrapping_sub(other);
                quotient.set_bit(i, true);
            }
        }
        Some((quotient, remainder))
    }

    pub fn udiv(&self, other: &Self) -> Option<Self> {
        self.udivrem(other).map(|(q, _)| q)
    }

    pub fn urem(&self, other: &Self) -> Option<Self> {
        self.udivrem(other).map(|(_, r)| r)
    }

    /// Signed quotient and remainder, truncating toward zero with the
    /// remainder taking the sign of the dividend, as LLVM's `sdiv`/`srem` do.
    pub fn sdivrem(&self, other: &Self) -> Option<(Self, Self)> {
        let negative_quotient = self.is_negative() != other.is_negative();
        let (q, r) = self.abs_signed().udivrem(&other.abs_signed())?;
        let q = if negative_quotient { q.negate() } else { q };
        let r = if self.is_negative() { r.negate() } else { r };
        Some((q, r))
    }

    pub fn sdiv(&self, other: &Self) -> Option<Self> {
        self.sdivrem(other).map(|(q, _)| q)
    }

    pub fn srem(&self, other: &Self) -> Option<Self> {
        self.sdivrem(other).map(|(_, r)| r)
    }

    pub fn cmp_unsigned(&self, other: &Self) -> Ordering {
        debug_assert_eq!(self.bits, other.bits, "width mismatch");
        let a = self.words();
        let b = other.words();
        for i in (0..a.len()).rev() {
            match a[i].cmp(&b[i]) {
                Ordering::Equal => {}
                non_equal => return non_equal,
            }
        }
        Ordering::Equal
    }

    pub fn cmp_signed(&self, other: &Self) -> Ordering {
        match (self.is_negative(), other.is_negative()) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => self.cmp_unsigned(other),
        }
    }

    pub fn count_ones(&self) -> u32 {
        self.words().iter().map(|w| w.count_ones()).sum()
    }

    pub fn leading_zeros(&self) -> u32 {
        let mut count = 0;
        for i in (0..self.bits).rev() {
            if self.bit(i) {
                break;
            }
            count += 1;
        }
        count
    }

    pub fn trailing_zeros(&self) -> u32 {
        let mut count = 0;
        for i in 0..self.bits {
            if self.bit(i) {
                break;
            }
            count += 1;
        }
        count
    }

    /// Divides by a single word, returning the quotient and the remainder.
    /// Used by decimal formatting, where the divisor is a power of ten.
    fn divrem_word(&self, divisor: u64) -> (Self, u64) {
        debug_assert_ne!(divisor, 0);
        let mut quotient = Self::new(self.bits);
        let mut remainder = 0u128;
        let d = u128::from(divisor);
        let n = self.words().len();
        for i in (0..n).rev() {
            let cur = (remainder << 64) | u128::from(self.words()[i]);
            quotient.words_mut()[i] = (cur / d) as u64;
            remainder = cur % d;
        }
        quotient.canonicalize();
        (quotient, remainder as u64)
    }

    /// Decimal digits of the value read as unsigned.
    pub fn to_string_unsigned(&self) -> String {
        if self.is_zero() {
            return "0".to_string();
        }
        // 10^19 is the largest power of ten that fits a u64, so each division
        // peels off 19 digits.
        const CHUNK: u64 = 10_000_000_000_000_000_000;
        let mut chunks = Vec::new();
        let mut value = self.clone();
        while !value.is_zero() {
            let (q, r) = value.divrem_word(CHUNK);
            chunks.push(r);
            value = q;
        }
        let mut out = String::new();
        out.push_str(&chunks[chunks.len() - 1].to_string());
        for chunk in chunks.iter().rev().skip(1) {
            out.push_str(&format!("{chunk:019}"));
        }
        out
    }

    /// Decimal digits of the value read as signed, with a leading minus when
    /// the sign bit is set. This is how LLVM prints integer constants.
    pub fn to_string_signed(&self) -> String {
        if self.is_negative() {
            format!("-{}", self.negate().to_string_unsigned())
        } else {
            self.to_string_unsigned()
        }
    }

    /// Hex digits without a `0x` prefix and without leading zeroes, which is
    /// the shape LLVM's float printer wants.
    pub fn to_hex_upper(&self) -> String {
        if self.is_zero() {
            return "0".to_string();
        }
        let words = self.words();
        let mut out = String::new();
        let mut started = false;
        for i in (0..words.len()).rev() {
            if !started {
                if words[i] == 0 {
                    continue;
                }
                out.push_str(&format!("{:X}", words[i]));
                started = true;
            } else {
                out.push_str(&format!("{:016X}", words[i]));
            }
        }
        out
    }

    /// Hex digits without a prefix, zero-padded to the full width.
    pub fn to_hex_upper_padded(&self) -> String {
        let words = self.words();
        let mut out = String::new();
        for i in (0..words.len()).rev() {
            out.push_str(&format!("{:016X}", words[i]));
        }
        let want = (self.bits.div_ceil(4)) as usize;
        if out.len() > want {
            out.split_off(out.len() - want)
        } else {
            out
        }
    }

    /// Reads digits in `radix` into the smallest width that holds them, with
    /// no sign handling. The caller applies the sign and the target width.
    pub fn parse_magnitude(digits: &str, radix: u32) -> Result<Self, ParseIntError> {
        debug_assert!((2..=16).contains(&radix));
        if digits.is_empty() {
            return Err(ParseIntError::Empty);
        }
        // Two extra words of headroom keep the running value from wrapping
        // before the final narrowing.
        let bits_per_digit = 32 - (radix - 1).leading_zeros();
        let width = (digits.len() as u32).saturating_mul(bits_per_digit).max(64) + 64;
        let mut value = Self::new(width);
        let radix_ap = Self::from_u64(width, u64::from(radix));
        for c in digits.chars() {
            let digit = c.to_digit(radix).ok_or(ParseIntError::InvalidDigit(c))?;
            value = value
                .wrapping_mul(&radix_ap)
                .wrapping_add(&Self::from_u64(width, u64::from(digit)));
        }
        Ok(value)
    }

    /// Reads an optionally signed integer literal into an exact width, the way
    /// the `.ll` parser needs it. A literal is in range when it fits the width
    /// either as a signed or as an unsigned value, matching LLVM.
    pub fn parse_sized(text: &str, radix: u32, bits: u32) -> Result<Self, ParseIntError> {
        let (negative, digits) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        let magnitude = Self::parse_magnitude(digits, radix)?;
        let width = magnitude.bits.max(bits + 1);
        let magnitude = magnitude.zext_or_trunc(width);
        let value = if negative {
            magnitude.negate()
        } else {
            magnitude.clone()
        };
        let narrowed = value.trunc(bits);
        // Round-tripping the narrowed value back to the wide one is the range
        // check: it holds for unsigned values that fit and for signed ones.
        let fits = if negative {
            narrowed.sext(width) == value
        } else {
            narrowed.zext(width) == value || narrowed.sext(width) == value
        };
        if !fits {
            return Err(ParseIntError::OutOfRange);
        }
        Ok(narrowed)
    }
}

impl PartialEq for ApInt {
    fn eq(&self, other: &Self) -> bool {
        self.bits == other.bits && self.words() == other.words()
    }
}

impl Eq for ApInt {}

impl Hash for ApInt {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bits.hash(state);
        self.words().hash(state);
    }
}

impl fmt::Debug for ApInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "i{}:{}", self.bits, self.to_string_signed())
    }
}

impl fmt::Display for ApInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_signed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic operand grid: a spread of hand-picked edge values plus a
    /// cheap linear-congruential sweep, so a failure reproduces exactly.
    fn operands() -> Vec<u128> {
        let mut values = vec![
            0,
            1,
            2,
            3,
            7,
            255,
            256,
            65535,
            u128::from(u32::MAX),
            u128::from(u32::MAX) + 1,
            u128::from(u64::MAX),
            u128::from(u64::MAX) + 1,
            u128::MAX,
            u128::MAX / 2,
            u128::MAX / 3,
            1 << 100,
        ];
        let mut state: u128 = 0x2545_F491_4F6C_DD1D;
        for _ in 0..64 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            values.push(state);
        }
        values
    }

    fn mask(bits: u32, v: u128) -> u128 {
        if bits >= 128 {
            v
        } else {
            v & ((1u128 << bits) - 1)
        }
    }

    fn as_signed(bits: u32, v: u128) -> i128 {
        let v = mask(bits, v);
        if bits < 128 && (v >> (bits - 1)) & 1 == 1 {
            (v as i128).wrapping_sub(1i128 << bits)
        } else {
            v as i128
        }
    }

    #[test]
    fn arithmetic_matches_native_u128() {
        for bits in [1u32, 7, 8, 16, 32, 63, 64, 65, 100, 127, 128] {
            for &a in &operands() {
                for &b in &operands() {
                    let (am, bm) = (mask(bits, a), mask(bits, b));
                    let x = ApInt::from_u128(bits, am);
                    let y = ApInt::from_u128(bits, bm);
                    assert_eq!(
                        x.wrapping_add(&y).to_u128_truncating(),
                        mask(bits, am.wrapping_add(bm)),
                        "add {am} + {bm} at i{bits}"
                    );
                    assert_eq!(
                        x.wrapping_sub(&y).to_u128_truncating(),
                        mask(bits, am.wrapping_sub(bm)),
                        "sub {am} - {bm} at i{bits}"
                    );
                    assert_eq!(
                        x.wrapping_mul(&y).to_u128_truncating(),
                        mask(bits, am.wrapping_mul(bm)),
                        "mul {am} * {bm} at i{bits}"
                    );
                    assert_eq!(x.and(&y).to_u128_truncating(), am & bm, "and at i{bits}");
                    assert_eq!(x.or(&y).to_u128_truncating(), am | bm, "or at i{bits}");
                    assert_eq!(x.xor(&y).to_u128_truncating(), am ^ bm, "xor at i{bits}");
                    assert_eq!(
                        x.cmp_unsigned(&y),
                        am.cmp(&bm),
                        "ucmp {am} vs {bm} at i{bits}"
                    );
                    assert_eq!(
                        x.cmp_signed(&y),
                        as_signed(bits, am).cmp(&as_signed(bits, bm)),
                        "scmp {am} vs {bm} at i{bits}"
                    );
                }
            }
        }
    }

    #[test]
    fn division_matches_native_u128() {
        for bits in [1u32, 8, 32, 64, 65, 128] {
            for &a in &operands() {
                for &b in &operands() {
                    let (am, bm) = (mask(bits, a), mask(bits, b));
                    let x = ApInt::from_u128(bits, am);
                    let y = ApInt::from_u128(bits, bm);
                    if bm == 0 {
                        assert!(x.udiv(&y).is_none(), "division by zero must fail");
                        continue;
                    }
                    assert_eq!(
                        x.udiv(&y).unwrap().to_u128_truncating(),
                        am / bm,
                        "udiv {am} / {bm} at i{bits}"
                    );
                    assert_eq!(
                        x.urem(&y).unwrap().to_u128_truncating(),
                        am % bm,
                        "urem {am} % {bm} at i{bits}"
                    );
                    let (asx, bsx) = (as_signed(bits, am), as_signed(bits, bm));
                    // The one case where the machine traps rather than wraps.
                    if bsx == -1 && asx == i128::MIN {
                        continue;
                    }
                    assert_eq!(
                        as_signed(bits, x.sdiv(&y).unwrap().to_u128_truncating()),
                        as_signed(bits, asx.wrapping_div(bsx) as u128),
                        "sdiv {asx} / {bsx} at i{bits}"
                    );
                    assert_eq!(
                        as_signed(bits, x.srem(&y).unwrap().to_u128_truncating()),
                        as_signed(bits, asx.wrapping_rem(bsx) as u128),
                        "srem {asx} % {bsx} at i{bits}"
                    );
                }
            }
        }
    }

    #[test]
    fn shifts_match_native_u128() {
        for bits in [1u32, 8, 32, 64, 65, 128] {
            for &a in &operands() {
                let am = mask(bits, a);
                let x = ApInt::from_u128(bits, am);
                for amount in [0u32, 1, 3, 7, 31, 32, 63, 64, 65, 100, 127, 128, 200] {
                    let expected_shl = if amount >= bits {
                        0
                    } else {
                        mask(bits, am << amount)
                    };
                    assert_eq!(
                        x.shl(amount).to_u128_truncating(),
                        expected_shl,
                        "shl {am} << {amount} at i{bits}"
                    );
                    let expected_lshr = if amount >= bits { 0 } else { am >> amount };
                    assert_eq!(
                        x.lshr(amount).to_u128_truncating(),
                        expected_lshr,
                        "lshr {am} >> {amount} at i{bits}"
                    );
                    let signed = as_signed(bits, am);
                    let expected_ashr = if amount >= bits {
                        if signed < 0 { mask(bits, u128::MAX) } else { 0 }
                    } else {
                        mask(bits, (signed >> amount) as u128)
                    };
                    assert_eq!(
                        x.ashr(amount).to_u128_truncating(),
                        expected_ashr,
                        "ashr {signed} >> {amount} at i{bits}"
                    );
                }
            }
        }
    }

    #[test]
    fn wide_values_exceed_two_words() {
        // 256 bits is past the inline representation, so this exercises the
        // heap path with the same algorithms.
        let a = ApInt::from_u128(256, u128::MAX);
        let b = a.wrapping_mul(&a);
        assert_eq!(b.bits(), 256);
        // (2^128 - 1)^2 = 2^256 - 2^129 + 1
        let expected = ApInt::from_u128(256, 1)
            .wrapping_sub(&ApInt::from_u128(256, 1).shl(129))
            .wrapping_add(&ApInt::new(256));
        assert_eq!(b, expected);
        assert_eq!(b.udiv(&a).unwrap(), a);
        assert!(b.urem(&a).unwrap().is_zero());
    }

    #[test]
    fn overflow_flags_match_native() {
        for bits in [8u32, 16, 32, 64] {
            for &a in &operands() {
                for &b in &operands() {
                    let (am, bm) = (mask(bits, a), mask(bits, b));
                    let x = ApInt::from_u128(bits, am);
                    let y = ApInt::from_u128(bits, bm);
                    let unsigned_max = mask(bits, u128::MAX);
                    assert_eq!(
                        x.carrying_add(&y).1,
                        am + bm > unsigned_max,
                        "uadd overflow {am} + {bm} at i{bits}"
                    );
                    assert_eq!(
                        x.borrowing_sub(&y).1,
                        am < bm,
                        "usub overflow {am} - {bm} at i{bits}"
                    );
                    assert_eq!(
                        x.unsigned_mul_overflows(&y),
                        am * bm > unsigned_max,
                        "umul overflow {am} * {bm} at i{bits}"
                    );
                    let (asx, bsx) = (as_signed(bits, am), as_signed(bits, bm));
                    let smin = -(1i128 << (bits - 1));
                    let smax = (1i128 << (bits - 1)) - 1;
                    assert_eq!(
                        x.signed_add_overflows(&y),
                        !(smin..=smax).contains(&(asx + bsx)),
                        "sadd overflow {asx} + {bsx} at i{bits}"
                    );
                    assert_eq!(
                        x.signed_sub_overflows(&y),
                        !(smin..=smax).contains(&(asx - bsx)),
                        "ssub overflow {asx} - {bsx} at i{bits}"
                    );
                    assert_eq!(
                        x.signed_mul_overflows(&y),
                        !(smin..=smax).contains(&(asx * bsx)),
                        "smul overflow {asx} * {bsx} at i{bits}"
                    );
                }
            }
        }
    }

    #[test]
    fn extension_and_truncation() {
        let v = ApInt::from_i128(8, -1);
        assert_eq!(v.to_u128_truncating(), 0xff);
        assert_eq!(v.sext(32).to_u128_truncating(), 0xffff_ffff);
        assert_eq!(v.zext(32).to_u128_truncating(), 0xff);
        assert_eq!(v.sext(32).trunc(8), v);
        assert!(v.is_negative());
        assert!(v.is_all_ones());
        assert_eq!(ApInt::from_i128(64, -2).to_i128(), Some(-2));
        assert_eq!(ApInt::signed_min(8).to_string_signed(), "-128");
        assert_eq!(ApInt::signed_max(8).to_string_signed(), "127");
        assert_eq!(ApInt::all_ones(1).to_string_signed(), "-1");
    }

    #[test]
    fn decimal_formatting_round_trips() {
        for bits in [8u32, 32, 64, 128, 256] {
            for &a in &operands() {
                let value = ApInt::from_u128(bits, mask(bits.min(128), a));
                let text = value.to_string_unsigned();
                let reparsed = ApInt::parse_magnitude(&text, 10)
                    .unwrap()
                    .zext_or_trunc(bits);
                assert_eq!(reparsed, value, "round trip of {text} at i{bits}");
            }
        }
        assert_eq!(ApInt::from_u64(64, 0).to_string_unsigned(), "0");
        assert_eq!(
            ApInt::all_ones(64).to_string_unsigned(),
            u64::MAX.to_string()
        );
        assert_eq!(
            ApInt::all_ones(128).to_string_unsigned(),
            u128::MAX.to_string()
        );
    }

    #[test]
    fn sized_parse_enforces_the_width() {
        assert_eq!(
            ApInt::parse_sized("255", 10, 8).unwrap().to_u64(),
            Some(255)
        );
        assert_eq!(
            ApInt::parse_sized("-128", 10, 8).unwrap().to_u64(),
            Some(128)
        );
        assert_eq!(ApInt::parse_sized("-1", 10, 8).unwrap().to_u64(), Some(255));
        assert_eq!(
            ApInt::parse_sized("256", 10, 8),
            Err(ParseIntError::OutOfRange)
        );
        assert_eq!(
            ApInt::parse_sized("-129", 10, 8),
            Err(ParseIntError::OutOfRange)
        );
        assert_eq!(ApInt::parse_sized("", 10, 8), Err(ParseIntError::Empty));
        assert_eq!(
            ApInt::parse_sized("12x", 10, 8),
            Err(ParseIntError::InvalidDigit('x'))
        );
        assert_eq!(ApInt::parse_sized("1", 10, 1).unwrap().to_u64(), Some(1));
        assert_eq!(ApInt::parse_sized("-1", 10, 1).unwrap().to_u64(), Some(1));
        assert_eq!(
            ApInt::parse_sized("2", 10, 1),
            Err(ParseIntError::OutOfRange)
        );
    }

    #[test]
    fn hex_formatting() {
        assert_eq!(ApInt::from_u64(64, 0).to_hex_upper(), "0");
        assert_eq!(ApInt::from_u64(64, 1).to_hex_upper(), "1");
        assert_eq!(
            ApInt::from_u64(64, 0x3FB9_9999_9999_999A).to_hex_upper(),
            "3FB999999999999A"
        );
        assert_eq!(ApInt::from_u64(16, 0x3C00).to_hex_upper_padded(), "3C00");
        assert_eq!(
            ApInt::from_u128(128, 1).to_hex_upper_padded(),
            "00000000000000000000000000000001"
        );
    }

    #[test]
    fn bit_counting() {
        let v = ApInt::from_u64(64, 0b1011_0000);
        assert_eq!(v.count_ones(), 3);
        assert_eq!(v.trailing_zeros(), 4);
        assert_eq!(v.leading_zeros(), 56);
        assert_eq!(ApInt::new(64).leading_zeros(), 64);
        assert_eq!(ApInt::new(64).trailing_zeros(), 64);
        assert_eq!(ApInt::all_ones(200).count_ones(), 200);
    }
}
