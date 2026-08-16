//! Bitstream reader for H.265/HEVC NAL unit parsing.
//!
//! Provides bit-level and exp-Golomb coded element reading from RBSP
//! (Raw Byte Sequence Payload) data.  This is the foundation for parsing
//! all HEVC syntax structures: VPS, SPS, PPS, slice headers, and SEI
//! messages.
//!
//! # HEVC bitstream conventions
//!
//! HEVC uses the same exp-Golomb and fixed-length coding as H.264 for
//! most syntax elements.  The descriptor notation in the spec is:
//!
//! * `u(n)` – unsigned integer using `n` bits
//! * `ue(v)` – unsigned exp-Golomb coded integer
//! * `se(v)` – signed exp-Golomb coded integer
//! * `f(n)` – fixed-pattern bit string of `n` bits
//!
//! # Emulation prevention
//!
//! The Annex B byte stream contains emulation prevention bytes (`0x03`)
//! that must be removed before parsing.  Use [`remove_emulation_prevention`]
//! to convert from SODB/encapsulated format to RBSP.

use crate::error::{DecodeError, DecodeResult};

/// A bit-level reader over RBSP (Raw Byte Sequence Payload) data.
///
/// Reads bits left-to-right (MSB first) within each byte, which matches
/// the HEVC bitstream convention.
///
/// # Example
///
/// ```ignore
/// use h265_decode::bitstream::BitstreamReader;
///
/// let data = [0b1010_0000];
/// let mut reader = BitstreamReader::new(&data);
/// assert_eq!(reader.read_bits(4).unwrap(), 0b1010);
/// ```
pub struct BitstreamReader<'a> {
    data: &'a [u8],
    byte_offset: usize,
    bit_offset: u8, // 0..=7, counts from MSB
}

impl<'a> BitstreamReader<'a> {
    /// Create a new reader over the given RBSP data.
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_offset: 0,
            bit_offset: 0,
        }
    }

    /// Returns the total number of bits remaining.
    #[inline]
    pub fn bits_remaining(&self) -> usize {
        if self.byte_offset >= self.data.len() {
            return 0;
        }
        (self.data.len() - self.byte_offset) * 8 - (self.bit_offset as usize)
    }

    /// Returns `true` if there are no more bits to read.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bits_remaining() == 0
    }

    /// Returns the current bit position (total bits consumed so far).
    #[inline]
    pub fn bit_position(&self) -> usize {
        self.byte_offset * 8 + self.bit_offset as usize
    }

    /// Read a single bit and return it as a `u8` (0 or 1).
    pub fn read_bit(&mut self) -> DecodeResult<u8> {
        if self.byte_offset >= self.data.len() {
            return Err(DecodeError::EndOfBitstream);
        }

        let byte = self.data[self.byte_offset];
        let bit = (byte >> (7 - self.bit_offset)) & 1;

        self.bit_offset += 1;
        if self.bit_offset == 8 {
            self.bit_offset = 0;
            self.byte_offset += 1;
        }

        Ok(bit)
    }

    /// Read a single bit as a boolean (`1` → `true`, `0` → `false`).
    #[inline]
    pub fn read_flag(&mut self) -> DecodeResult<bool> {
        Ok(self.read_bit()? != 0)
    }

    /// Read `n` bits as an unsigned integer (MSB first).
    ///
    /// `n` must be in the range `0..=32`.  Reading 0 bits always returns 0.
    pub fn read_bits(&mut self, n: u8) -> DecodeResult<u32> {
        debug_assert!(n <= 32, "read_bits: n={n} exceeds 32");

        if n == 0 {
            return Ok(0);
        }

        let mut value: u32 = 0;
        for _ in 0..n {
            value = (value << 1) | (self.read_bit()? as u32);
        }
        Ok(value)
    }

    /// Read `n` bits as an unsigned 64-bit integer (MSB first).
    ///
    /// `n` must be in the range `0..=64`.
    pub fn read_bits_u64(&mut self, n: u8) -> DecodeResult<u64> {
        debug_assert!(n <= 64, "read_bits_u64: n={n} exceeds 64");

        if n == 0 {
            return Ok(0);
        }

        let mut value: u64 = 0;
        for _ in 0..n {
            value = (value << 1) | (self.read_bit()? as u64);
        }
        Ok(value)
    }

    /// Read an unsigned exp-Golomb coded integer (`ue(v)` in the spec).
    ///
    /// The coding is:
    /// 1. Count the number of leading zero bits (`leadingZeroBits`).
    /// 2. Read `leadingZeroBits` more bits as the suffix.
    /// 3. Value = `(1 << leadingZeroBits) - 1 + suffix`.
    ///
    /// Examples: `1` → 0, `010` → 1, `011` → 2, `00100` → 3, etc.
    pub fn read_ue(&mut self) -> DecodeResult<u32> {
        let mut leading_zeros: u32 = 0;

        loop {
            let bit = self.read_bit()?;
            if bit == 1 {
                break;
            }
            leading_zeros += 1;

            // Safety limit: exp-Golomb values > 2^31 are unreasonable
            // for any HEVC syntax element.
            if leading_zeros > 31 {
                return Err(DecodeError::InvalidBitstream(
                    "exp-Golomb code too long (>31 leading zeros)".into(),
                ));
            }
        }

        if leading_zeros == 0 {
            return Ok(0);
        }

        let suffix = self.read_bits(leading_zeros as u8)?;
        Ok((1u32 << leading_zeros) - 1 + suffix)
    }

    /// Read a signed exp-Golomb coded integer (`se(v)` in the spec).
    ///
    /// Maps unsigned code values to signed values:
    /// `0 → 0, 1 → 1, 2 → -1, 3 → 2, 4 → -2, ...`
    ///
    /// Formula: if `k = ue(v)`, then `se(v) = ceil(k/2) * (-1)^(k+1)`.
    pub fn read_se(&mut self) -> DecodeResult<i32> {
        let code = self.read_ue()?;
        let abs_val = ((code + 1) >> 1) as i32;
        if code & 1 == 0 {
            // Even code → negative
            Ok(-abs_val)
        } else {
            // Odd code → positive
            Ok(abs_val)
        }
    }

    /// Skip `n` bits without returning a value.
    pub fn skip_bits(&mut self, n: u32) -> DecodeResult<()> {
        for _ in 0..n {
            self.read_bit()?;
        }
        Ok(())
    }

    /// Skip bits until the reader is aligned to a byte boundary.
    ///
    /// If already aligned, this is a no-op.
    pub fn align_to_byte(&mut self) -> DecodeResult<()> {
        if self.bit_offset != 0 {
            let skip = 8 - self.bit_offset as u32;
            self.skip_bits(skip)?;
        }
        Ok(())
    }

    /// Read a `u(1)` flag that the HEVC spec says should be zero.
    ///
    /// Returns the actual value but logs a warning if it is non-zero.
    pub fn read_zero_bit(&mut self) -> DecodeResult<u8> {
        let bit = self.read_bit()?;
        if bit != 0 {
            log::warn!(
                "expected zero bit at position {}, got 1",
                self.bit_position() - 1
            );
        }
        Ok(bit)
    }

    /// Check whether there is more RBSP data before the trailing bits.
    ///
    /// This implements the `more_rbsp_data()` function from the spec:
    /// returns `true` if there is at least one more non-zero bit before
    /// the RBSP stop bit and trailing alignment zeros.
    pub fn more_rbsp_data(&self) -> bool {
        if self.byte_offset >= self.data.len() {
            return false;
        }

        // Find the last non-zero byte
        let mut last_nonzero = self.data.len();
        while last_nonzero > self.byte_offset {
            last_nonzero -= 1;
            if self.data[last_nonzero] != 0 {
                break;
            }
        }

        if last_nonzero < self.byte_offset {
            return false;
        }

        // Find the position of the RBSP stop bit (the highest set bit in the
        // last non-zero byte).
        let last_byte = self.data[last_nonzero];
        let stop_bit_pos = last_nonzero * 8 + (7 - (last_byte.trailing_zeros() as usize));

        // There is more data if our current position is before the stop bit.
        self.bit_position() < stop_bit_pos
    }
}

/// Remove emulation prevention bytes (`0x00 0x00 0x03`) from encapsulated
/// NAL unit data to produce the Raw Byte Sequence Payload (RBSP).
///
/// In the Annex B byte stream and in length-delimited NAL units, the
/// sequence `0x00 0x00 0x03` is an escape that represents `0x00 0x00` in
/// the RBSP.  This function strips the `0x03` bytes.
///
/// Per the spec, `0x03` is inserted before `0x00`, `0x01`, `0x02`, and
/// `0x03` when preceded by `0x00 0x00`.
pub fn remove_emulation_prevention(data: &[u8]) -> Vec<u8> {
    let mut rbsp = Vec::with_capacity(data.len());
    let mut i = 0;

    while i < data.len() {
        if i + 2 < data.len() && data[i] == 0x00 && data[i + 1] == 0x00 && data[i + 2] == 0x03 {
            // Emit the two zero bytes, skip the 0x03 prevention byte.
            rbsp.push(0x00);
            rbsp.push(0x00);
            i += 3;
        } else {
            rbsp.push(data[i]);
            i += 1;
        }
    }

    rbsp
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- BitstreamReader basic tests --

    #[test]
    fn test_read_single_bits() {
        let data = [0b1010_0110];
        let mut r = BitstreamReader::new(&data);

        assert_eq!(r.read_bit().unwrap(), 1);
        assert_eq!(r.read_bit().unwrap(), 0);
        assert_eq!(r.read_bit().unwrap(), 1);
        assert_eq!(r.read_bit().unwrap(), 0);
        assert_eq!(r.read_bit().unwrap(), 0);
        assert_eq!(r.read_bit().unwrap(), 1);
        assert_eq!(r.read_bit().unwrap(), 1);
        assert_eq!(r.read_bit().unwrap(), 0);

        assert!(r.read_bit().is_err());
    }

    #[test]
    fn test_read_flag() {
        let data = [0b1000_0000];
        let mut r = BitstreamReader::new(&data);
        assert!(r.read_flag().unwrap());

        let data = [0b0000_0000];
        let mut r = BitstreamReader::new(&data);
        assert!(!r.read_flag().unwrap());
    }

    #[test]
    fn test_read_bits_4() {
        let data = [0b1011_0100];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_bits(4).unwrap(), 0b1011);
        assert_eq!(r.read_bits(4).unwrap(), 0b0100);
    }

    #[test]
    fn test_read_bits_across_bytes() {
        let data = [0b1111_0000, 0b1010_0101];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_bits(4).unwrap(), 0b1111);
        assert_eq!(r.read_bits(8).unwrap(), 0b0000_1010);
        assert_eq!(r.read_bits(4).unwrap(), 0b0101);
    }

    #[test]
    fn test_read_bits_zero() {
        let data = [0xFF];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_bits(0).unwrap(), 0);
        assert_eq!(r.bits_remaining(), 8);
    }

    #[test]
    fn test_read_bits_full_32() {
        let data = [0x12, 0x34, 0x56, 0x78];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_bits(32).unwrap(), 0x12345678);
    }

    #[test]
    fn test_read_bits_u64() {
        let data = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_bits_u64(64).unwrap(), 0x123456789ABCDEF0);
    }

    // -- Exp-Golomb tests --

    #[test]
    fn test_ue_zero() {
        // ue(0) = "1" = bit pattern: 1
        let data = [0b1000_0000];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_ue().unwrap(), 0);
    }

    #[test]
    fn test_ue_one() {
        // ue(1) = "010" = bit pattern: 010
        let data = [0b0100_0000];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_ue().unwrap(), 1);
    }

    #[test]
    fn test_ue_two() {
        // ue(2) = "011" = bit pattern: 011
        let data = [0b0110_0000];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_ue().unwrap(), 2);
    }

    #[test]
    fn test_ue_three() {
        // ue(3) = "00100" = bit pattern: 00100
        let data = [0b0010_0000];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_ue().unwrap(), 3);
    }

    #[test]
    fn test_ue_four() {
        // ue(4) = "00101"
        let data = [0b0010_1000];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_ue().unwrap(), 4);
    }

    #[test]
    fn test_ue_five() {
        // ue(5) = "00110"
        let data = [0b0011_0000];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_ue().unwrap(), 5);
    }

    #[test]
    fn test_ue_six() {
        // ue(6) = "00111"
        let data = [0b0011_1000];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_ue().unwrap(), 6);
    }

    #[test]
    fn test_ue_seven() {
        // ue(7) = "0001000" (3 leading zeros, 1, suffix=000)
        let data = [0b0001_0000];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_ue().unwrap(), 7);
    }

    #[test]
    fn test_ue_sequential() {
        // ue(0)=1, ue(3)=00100, ue(1)=010 → 1 00100 010 = 0b1_00100_01 0...
        // byte 0: 1_00100_01 = 0b10010001 = 0x91
        // byte 1: 0_0000000  = 0b00000000
        let data = [0b1001_0001, 0b0000_0000];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_ue().unwrap(), 0);
        assert_eq!(r.read_ue().unwrap(), 3);
        assert_eq!(r.read_ue().unwrap(), 1);
    }

    // -- Signed exp-Golomb tests --

    #[test]
    fn test_se_zero() {
        // se(0): ue=0 → 0
        let data = [0b1000_0000];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_se().unwrap(), 0);
    }

    #[test]
    fn test_se_positive_one() {
        // se(1): ue=1 → +1 (odd code)
        let data = [0b0100_0000];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_se().unwrap(), 1);
    }

    #[test]
    fn test_se_negative_one() {
        // se(-1): ue=2 → -1 (even code)
        let data = [0b0110_0000];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_se().unwrap(), -1);
    }

    #[test]
    fn test_se_positive_two() {
        // se(2): ue=3 → +2 (odd code)
        let data = [0b0010_0000];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_se().unwrap(), 2);
    }

    #[test]
    fn test_se_negative_two() {
        // se(-2): ue=4 → -2 (even code)
        let data = [0b0010_1000];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_se().unwrap(), -2);
    }

    // -- Skip and alignment tests --

    #[test]
    fn test_skip_bits() {
        let data = [0b1111_0000, 0b1010_0101];
        let mut r = BitstreamReader::new(&data);
        r.skip_bits(4).unwrap();
        assert_eq!(r.read_bits(4).unwrap(), 0b0000);
    }

    #[test]
    fn test_align_to_byte() {
        let data = [0b1010_0110, 0b1111_0000];
        let mut r = BitstreamReader::new(&data);
        r.read_bits(3).unwrap(); // consume 3 bits
        r.align_to_byte().unwrap(); // skip remaining 5 bits
        assert_eq!(r.bit_position(), 8);
        assert_eq!(r.read_bits(8).unwrap(), 0b1111_0000);
    }

    #[test]
    fn test_align_already_aligned() {
        let data = [0xFF, 0x00];
        let mut r = BitstreamReader::new(&data);
        r.read_bits(8).unwrap();
        r.align_to_byte().unwrap(); // no-op
        assert_eq!(r.bit_position(), 8);
    }

    // -- bits_remaining / is_empty --

    #[test]
    fn test_bits_remaining() {
        let data = [0xFF, 0x00];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.bits_remaining(), 16);
        assert!(!r.is_empty());

        r.read_bits(5).unwrap();
        assert_eq!(r.bits_remaining(), 11);

        r.read_bits(11).unwrap();
        assert_eq!(r.bits_remaining(), 0);
        assert!(r.is_empty());
    }

    #[test]
    fn test_empty_reader() {
        let data: &[u8] = &[];
        let r = BitstreamReader::new(data);
        assert!(r.is_empty());
        assert_eq!(r.bits_remaining(), 0);
    }

    #[test]
    fn test_read_beyond_end() {
        let data = [0xFF];
        let mut r = BitstreamReader::new(&data);
        r.read_bits(8).unwrap();
        assert!(r.read_bit().is_err());
    }

    // -- more_rbsp_data --

    #[test]
    fn test_more_rbsp_data_with_trailing() {
        // Data: 0xFF, 0x80 (= 1000_0000 which is just a stop bit)
        let data = [0xFF, 0x80];
        let mut r = BitstreamReader::new(&data);
        assert!(r.more_rbsp_data());

        r.read_bits(8).unwrap(); // consume first byte
        // Now at the stop bit byte; the stop bit itself means no more data
        assert!(!r.more_rbsp_data());
    }

    #[test]
    fn test_more_rbsp_data_real_data_then_stop() {
        // Data byte, then stop bit: 0xAB, 0b1_1000000
        // After consuming 0xAB, there is still a '1' bit before the stop bit
        let data = [0xAB, 0b1100_0000];
        let mut r = BitstreamReader::new(&data);
        r.read_bits(8).unwrap(); // consume 0xAB
        // Position at bit 8. Last non-zero byte is index 1 = 0b1100_0000.
        // Trailing zeros: 6, so stop bit is at bit 8 + (7 - 6) = 9.
        // We are at bit 8 < 9, so there is more data.
        assert!(r.more_rbsp_data());
    }

    // -- bit_position --

    #[test]
    fn test_bit_position() {
        let data = [0xFF, 0x00, 0xAA];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.bit_position(), 0);

        r.read_bits(3).unwrap();
        assert_eq!(r.bit_position(), 3);

        r.read_bits(8).unwrap();
        assert_eq!(r.bit_position(), 11);
    }

    // -- Emulation prevention removal --

    #[test]
    fn test_remove_emulation_prevention_none() {
        let data = [0x01, 0x02, 0x03, 0x04];
        let rbsp = remove_emulation_prevention(&data);
        assert_eq!(rbsp, data);
    }

    #[test]
    fn test_remove_emulation_prevention_basic() {
        // 00 00 03 → 00 00
        let data = [0x00, 0x00, 0x03, 0x00];
        let rbsp = remove_emulation_prevention(&data);
        assert_eq!(rbsp, [0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_remove_emulation_prevention_multiple() {
        let data = [0x00, 0x00, 0x03, 0x01, 0x00, 0x00, 0x03, 0x02];
        let rbsp = remove_emulation_prevention(&data);
        assert_eq!(rbsp, [0x00, 0x00, 0x01, 0x00, 0x00, 0x02]);
    }

    #[test]
    fn test_remove_emulation_prevention_at_end() {
        let data = [0xAA, 0x00, 0x00, 0x03];
        let rbsp = remove_emulation_prevention(&data);
        assert_eq!(rbsp, [0xAA, 0x00, 0x00]);
    }

    #[test]
    fn test_remove_emulation_prevention_consecutive() {
        // Two prevention bytes back-to-back
        let data = [0x00, 0x00, 0x03, 0x00, 0x00, 0x03, 0x03];
        let rbsp = remove_emulation_prevention(&data);
        assert_eq!(rbsp, [0x00, 0x00, 0x00, 0x00, 0x03]);
    }

    #[test]
    fn test_remove_emulation_prevention_empty() {
        let rbsp = remove_emulation_prevention(&[]);
        assert!(rbsp.is_empty());
    }

    #[test]
    fn test_remove_emulation_prevention_short() {
        let rbsp = remove_emulation_prevention(&[0x00, 0x00]);
        assert_eq!(rbsp, [0x00, 0x00]);
    }

    // -- read_zero_bit --

    #[test]
    fn test_read_zero_bit_actual_zero() {
        let data = [0b0000_0000];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_zero_bit().unwrap(), 0);
    }

    #[test]
    fn test_read_zero_bit_actual_one() {
        let data = [0b1000_0000];
        let mut r = BitstreamReader::new(&data);
        // Should return 1 (with a warning log, but no error)
        assert_eq!(r.read_zero_bit().unwrap(), 1);
    }

    // -- Edge cases --

    #[test]
    fn test_ue_large_value() {
        // ue with 8 leading zeros: 0000_0000 1 XXXX_XXXX
        // Value = (1 << 8) - 1 + suffix = 255 + suffix
        // Let suffix = 0b0000_0000 = 0 → value = 255
        let data = [0b0000_0000, 0b1000_0000, 0b0000_0000];
        let mut r = BitstreamReader::new(&data);
        assert_eq!(r.read_ue().unwrap(), 255);
    }

    #[test]
    fn test_ue_too_many_leading_zeros() {
        // 32+ leading zeros should fail
        let data = [0x00, 0x00, 0x00, 0x00, 0x01];
        let mut r = BitstreamReader::new(&data);
        assert!(r.read_ue().is_err());
    }

    #[test]
    fn test_se_mapping_table() {
        // Verify the complete mapping for small values:
        // ue=0 → se=0, ue=1 → se=1, ue=2 → se=-1, ue=3 → se=2, ue=4 → se=-2
        let expected: Vec<(u32, i32)> =
            vec![(0, 0), (1, 1), (2, -1), (3, 2), (4, -2), (5, 3), (6, -3)];
        for (ue_val, expected_se) in expected {
            let abs_val = ((ue_val + 1) >> 1) as i32;
            let se = if ue_val & 1 == 0 { -abs_val } else { abs_val };
            assert_eq!(
                se, expected_se,
                "ue={ue_val} should map to se={expected_se}"
            );
        }
    }
}
