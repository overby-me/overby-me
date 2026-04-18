//! LZW decompressor for Unix `compress` format (magic `1f 9d`).
//!
//! This is a Rust port of GNU gzip's `unlzw.c`.

use std::io::{self, Write};

/// Maximum bits supported by this implementation.
const BITS: usize = 16;
/// Initial code width.
const INIT_BITS: usize = 9;
/// Mask to extract maxbits from the flags byte (bits 0..4).
const BIT_MASK: u8 = 0x1f;
/// If set in the flags byte, CLEAR code (256) resets the table.
const BLOCK_MODE: u8 = 0x80;
/// Reserved bits in the flags byte (bits 5..6).
const LZW_RESERVED: u8 = 0x60;
/// The CLEAR code that resets the string table.
const CLEAR: usize = 256;
/// First free table entry (after CLEAR).
const FIRST: usize = CLEAR + 1; // 257

/// Read an `n_bits`-wide code from `buf` at bit position `posbits` (LSB-first).
///
/// Reads 3 bytes starting at `posbits >> 3`, combines them as a 24-bit
/// little-endian value, shifts right by `posbits & 7`, and masks with
/// `bitmask`.  Returns the code value.
#[inline]
fn read_code(buf: &[u8], posbits: usize, bitmask: usize) -> usize {
    let byte_pos = posbits >> 3;
    let bit_off = posbits & 7;
    // Safely read up to 3 bytes, treating out-of-bounds as 0.
    let b0 = *buf.get(byte_pos).unwrap_or(&0) as usize;
    let b1 = *buf.get(byte_pos + 1).unwrap_or(&0) as usize;
    let b2 = *buf.get(byte_pos + 2).unwrap_or(&0) as usize;
    ((b0 | (b1 << 8) | (b2 << 16)) >> bit_off) & bitmask
}

/// Align `posbits` forward to the next `n_bits`-code boundary.
///
/// Mirrors the C expression:
///
/// ```text
/// posbits = ((posbits-1) + ((n_bits<<3) - (posbits-1+(n_bits<<3))%(n_bits<<3)));
/// ```
#[inline]
fn align_to_code_boundary(posbits: usize, n_bits: usize) -> usize {
    let chunk = n_bits << 3; // n_bits codes are packed per byte-group
    let p = posbits.wrapping_sub(1);
    p + (chunk - (p + chunk) % chunk)
}
/// Decompress data in Unix `compress` (LZW) format.
///
/// `data` is everything AFTER the 2-byte magic (`1f 9d`). The first byte
/// of `data` is the flags/maxbits byte:
///   - bits 0..4 (`BIT_MASK` = 0x1f): max bits per code (9..=16 typically)
///   - bit 7 (`BLOCK_MODE` = 0x80): if set, CLEAR code (256) resets the table
///   - bits 5..6 (`LZW_RESERVED` = 0x60): reserved, warn if set but proceed
///
/// After that first byte, the rest is the LZW-coded bitstream.
pub fn decompress_lzw<W: Write>(data: &[u8], writer: &mut W) -> io::Result<()> {
    if data.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "corrupt input."));
    }

    // -- Parse flags byte -------------------------------------------------
    let flags = data[0];
    let maxbits = (flags & BIT_MASK) as usize;
    let block_mode = (flags & BLOCK_MODE) != 0;

    if maxbits > BITS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "compressed with {} bits, can only handle {} bits",
                maxbits, BITS
            ),
        ));
    }

    // Reserved bits -- the original warns but continues; we silently proceed.
    let _reserved = (flags & LZW_RESERVED) != 0;

    let maxmaxcode: usize = 1 << maxbits;

    // -- String table -----------------------------------------------------
    let mut tab_prefix: Vec<u16> = vec![0u16; maxmaxcode];
    let mut tab_suffix: Vec<u8> = vec![0u8; maxmaxcode];

    // Initialise literal entries.
    for i in 0..=255u16 {
        tab_suffix[i as usize] = i as u8;
    }

    // -- State variables --------------------------------------------------
    let mut n_bits: usize = INIT_BITS;
    let mut maxcode: usize = (1 << n_bits) - 1;
    let mut bitmask: usize = (1 << n_bits) - 1;
    let mut free_ent: usize = if block_mode { FIRST } else { CLEAR };
    let mut oldcode: isize = -1; // -1 means "no previous code yet"
    let mut finchar: u8 = 0;

    // The compressed bitstream starts at data[1..].
    // We track positions in *bits* relative to data[1].
    let inbuf = &data[1..];
    let insize = inbuf.len(); // number of available bytes
    let mut posbits: usize = 0; // current bit position in inbuf

    // Stack for reversing decoded strings.
    let mut stack: Vec<u8> = Vec::with_capacity(8192);

    // Output buffer -- we flush in decent-sized chunks.
    let mut outbuf: Vec<u8> = Vec::with_capacity(16384);
    // -- Main decompression loop ------------------------------------------
    //
    // The C code has a resetbuf label that the inner loop jumps to
    // after a CLEAR or a code-width increase.  In Rust we model this as
    // a labeled outer loop that re-enters the inner while-loop.
    'resetbuf: loop {
        // Compute the effective bit limit following the C code:
        //   e = insize - (o = posbits >> 3);
        //   e = (e << 3) - (n_bits - 1);
        // This ensures we never try to read a partial code at the tail.
        let inbits = {
            let o = posbits >> 3;
            if o >= insize {
                break; // no more input
            }
            let bytes_left = insize - o;
            let e = (bytes_left << 3).saturating_sub(n_bits - 1);
            (posbits & !7) + e
        };

        while posbits < inbits {
            // If the table is full at the current width, bump n_bits.
            if free_ent > maxcode {
                posbits = align_to_code_boundary(posbits, n_bits);
                n_bits += 1;
                if n_bits > maxbits {
                    n_bits = maxbits; // clamp
                }
                maxcode = if n_bits == maxbits {
                    maxmaxcode
                } else {
                    (1 << n_bits) - 1
                };
                bitmask = (1 << n_bits) - 1;
                continue 'resetbuf;
            }

            // -- Read one code ------------------------------------------------
            let code = read_code(inbuf, posbits, bitmask);
            posbits += n_bits;

            // -- First code ---------------------------------------------------
            if oldcode == -1 {
                if code >= 256 {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "corrupt input."));
                }
                finchar = code as u8;
                oldcode = code as isize;
                outbuf.push(finchar);
                if outbuf.len() >= 16384 {
                    writer.write_all(&outbuf)?;
                    outbuf.clear();
                }
                continue;
            }

            // -- CLEAR code ---------------------------------------------------
            if code == CLEAR && block_mode {
                // Clear the prefix table.
                for p in tab_prefix.iter_mut() {
                    *p = 0;
                }
                free_ent = FIRST - 1;
                posbits = align_to_code_boundary(posbits, n_bits);
                n_bits = INIT_BITS;
                maxcode = (1 << n_bits) - 1;
                bitmask = (1 << n_bits) - 1;
                continue 'resetbuf;
            }
            // -- Normal code --------------------------------------------------
            let incode = code;
            let mut code = code;

            stack.clear();

            // Handle the KwKwK special case.
            if code >= free_ent {
                if code > free_ent {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "corrupt input."));
                }
                stack.push(finchar);
                code = oldcode as usize;
            }

            // Walk the chain to decode the string (reversed).
            while code >= 256 {
                if code >= maxmaxcode {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "corrupt input."));
                }
                stack.push(tab_suffix[code]);
                code = tab_prefix[code] as usize;
            }

            // code is now a literal byte.
            finchar = tab_suffix[code]; // == code as u8
            stack.push(finchar);

            // Output the decoded string (stack is reversed, so iterate backwards).
            for &b in stack.iter().rev() {
                outbuf.push(b);
            }
            if outbuf.len() >= 16384 {
                writer.write_all(&outbuf)?;
                outbuf.clear();
            }

            // -- Add new entry to the string table ----------------------------
            if free_ent < maxmaxcode {
                tab_prefix[free_ent] = oldcode as u16;
                tab_suffix[free_ent] = finchar;
                free_ent += 1;
            }

            oldcode = incode as isize;
        }

        // If the inner loop exhausted inbits without needing a resetbuf,
        // we are done.
        break;
    }

    // -- Flush remaining output -------------------------------------------
    if !outbuf.is_empty() {
        writer.write_all(&outbuf)?;
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: write a value of n_bits width at bit_pos in buf (LSB-first).
    fn write_code(buf: &mut Vec<u8>, bit_pos: &mut usize, value: usize, n_bits: usize) {
        let need_bytes = (*bit_pos + n_bits + 7) / 8;
        if buf.len() < need_bytes {
            buf.resize(need_bytes, 0);
        }
        for i in 0..n_bits {
            if (value >> i) & 1 != 0 {
                let byte_idx = (*bit_pos + i) >> 3;
                let bit_idx = (*bit_pos + i) & 7;
                if byte_idx >= buf.len() {
                    buf.push(0);
                }
                buf[byte_idx] |= 1 << bit_idx;
            }
        }
        *bit_pos += n_bits;
        let final_bytes = (*bit_pos + 7) / 8;
        if buf.len() < final_bytes {
            buf.resize(final_bytes, 0);
        }
    }

    #[test]
    fn test_single_literal() {
        let flags: u8 = 0x80 | 9;
        let mut bitstream = Vec::new();
        let mut bp = 0usize;
        write_code(&mut bitstream, &mut bp, 0x41, 9);

        let mut data = vec![flags];
        data.extend_from_slice(&bitstream);

        let mut output = Vec::new();
        decompress_lzw(&data, &mut output).unwrap();
        assert_eq!(output, b"A");
    }

    #[test]
    fn test_literal_sequence() {
        let flags: u8 = 0x80 | 9;
        let mut bitstream = Vec::new();
        let mut bp = 0usize;
        write_code(&mut bitstream, &mut bp, b'A' as usize, 9);
        write_code(&mut bitstream, &mut bp, b'B' as usize, 9);
        write_code(&mut bitstream, &mut bp, b'C' as usize, 9);

        let mut data = vec![flags];
        data.extend_from_slice(&bitstream);

        let mut output = Vec::new();
        decompress_lzw(&data, &mut output).unwrap();
        assert_eq!(output, b"ABC");
    }

    #[test]
    fn test_kwkwk_case() {
        // AAA via KwKwK: emit A then 257 (== free_ent)
        let flags: u8 = 0x80 | 9;
        let mut bitstream = Vec::new();
        let mut bp = 0usize;
        write_code(&mut bitstream, &mut bp, b'A' as usize, 9);
        write_code(&mut bitstream, &mut bp, 257, 9);

        let mut data = vec![flags];
        data.extend_from_slice(&bitstream);

        let mut output = Vec::new();
        decompress_lzw(&data, &mut output).unwrap();
        assert_eq!(output, b"AAA");
    }

    #[test]
    fn test_maxbits_too_large() {
        let flags: u8 = 0x80 | 17;
        let data = vec![flags, 0];

        let mut output = Vec::new();
        let err = decompress_lzw(&data, &mut output).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        let msg = err.to_string();
        assert!(msg.contains("17 bits"), "message was: {}", msg);
        assert!(msg.contains("16 bits"), "message was: {}", msg);
    }

    #[test]
    fn test_empty_data() {
        let mut output = Vec::new();
        let err = decompress_lzw(&[], &mut output).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn test_first_code_not_literal() {
        let flags: u8 = 0x80 | 9;
        let mut bitstream = Vec::new();
        let mut bp = 0usize;
        write_code(&mut bitstream, &mut bp, 257, 9);

        let mut data = vec![flags];
        data.extend_from_slice(&bitstream);

        let mut output = Vec::new();
        let err = decompress_lzw(&data, &mut output).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("corrupt input."));
    }

    #[test]
    fn test_clear_code() {
        // Encode: A, B, CLEAR, C -- after CLEAR table resets
        let flags: u8 = 0x80 | 9;
        let mut bitstream = Vec::new();
        let mut bp = 0usize;
        write_code(&mut bitstream, &mut bp, b'A' as usize, 9);
        write_code(&mut bitstream, &mut bp, b'B' as usize, 9);
        write_code(&mut bitstream, &mut bp, CLEAR, 9);
        // After CLEAR, bits are realigned to a 9-code boundary.
        bp = align_to_code_boundary(bp, 9);
        let need = (bp + 7) / 8;
        if bitstream.len() < need {
            bitstream.resize(need, 0);
        }
        write_code(&mut bitstream, &mut bp, b'C' as usize, 9);

        let mut data = vec![flags];
        data.extend_from_slice(&bitstream);

        let mut output = Vec::new();
        decompress_lzw(&data, &mut output).unwrap();
        assert_eq!(output, b"ABC");
    }

    #[test]
    fn test_table_entry_reuse() {
        // Encode ABAB using table entry 257 = (A, B)
        let flags: u8 = 0x80 | 9;
        let mut bitstream = Vec::new();
        let mut bp = 0usize;
        write_code(&mut bitstream, &mut bp, b'A' as usize, 9);
        write_code(&mut bitstream, &mut bp, b'B' as usize, 9);
        write_code(&mut bitstream, &mut bp, 257, 9);

        let mut data = vec![flags];
        data.extend_from_slice(&bitstream);

        let mut output = Vec::new();
        decompress_lzw(&data, &mut output).unwrap();
        assert_eq!(output, b"ABAB");
    }

    #[test]
    fn test_no_block_mode() {
        // Without block_mode, free_ent starts at 256 (not 257).
        let flags: u8 = 9; // block_mode=0, maxbits=9
        let mut bitstream = Vec::new();
        let mut bp = 0usize;
        write_code(&mut bitstream, &mut bp, b'X' as usize, 9);
        write_code(&mut bitstream, &mut bp, b'Y' as usize, 9);

        let mut data = vec![flags];
        data.extend_from_slice(&bitstream);

        let mut output = Vec::new();
        decompress_lzw(&data, &mut output).unwrap();
        assert_eq!(output, b"XY");
    }

    #[test]
    fn test_code_greater_than_free_ent() {
        // Code > free_ent should be rejected as corrupt.
        let flags: u8 = 0x80 | 9;
        let mut bitstream = Vec::new();
        let mut bp = 0usize;
        write_code(&mut bitstream, &mut bp, b'A' as usize, 9);
        write_code(&mut bitstream, &mut bp, 258, 9); // free_ent is 257

        let mut data = vec![flags];
        data.extend_from_slice(&bitstream);

        let mut output = Vec::new();
        let err = decompress_lzw(&data, &mut output).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("corrupt input."));
    }

    #[test]
    fn test_only_flags_byte_no_data() {
        let flags: u8 = 0x80 | 9;
        let data = vec![flags];

        let mut output = Vec::new();
        decompress_lzw(&data, &mut output).unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn test_reserved_bits_set() {
        // Reserved bits set should not cause an error.
        let flags: u8 = 0x80 | 0x60 | 9; // block_mode + reserved + maxbits=9
        let mut bitstream = Vec::new();
        let mut bp = 0usize;
        write_code(&mut bitstream, &mut bp, b'Z' as usize, 9);

        let mut data = vec![flags];
        data.extend_from_slice(&bitstream);

        let mut output = Vec::new();
        decompress_lzw(&data, &mut output).unwrap();
        assert_eq!(output, b"Z");
    }

    #[test]
    fn test_real_compress_interop() {
        // "hello\n" compressed with Unix compress (after stripping 1f 9d magic)
        // 0x90 = 0x80 | 0x10 = block_mode + maxbits=16
        let payload: &[u8] = &[0x90, 0x68, 0xca, 0xb0, 0x61, 0xf3, 0x46, 0x01];

        let mut output = Vec::new();
        decompress_lzw(payload, &mut output).unwrap();
        assert_eq!(output, b"hello\n");
    }
}
