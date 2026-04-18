//! Pack format decompressor (magic `1f 1e`).
//!
//! This is a Rust port of GNU gzip's `unpack.c`. The pack format uses a
//! static Huffman tree described in the file header, followed by an
//! MSB-first bitstream of Huffman codes.

use std::io::{self, Write};

const MAX_BITLEN: usize = 25;
const LITERALS: usize = 256;
const MAX_PEEK: usize = 12;

/// MSB-first bit reader over a byte slice.
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bitbuf: u64,
    valid: i32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            bitbuf: 0,
            valid: 0,
        }
    }

    fn read_byte(&mut self) -> io::Result<u8> {
        if self.pos >= self.data.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid compressed data -- unexpected end of file",
            ));
        }
        let b = self.data[self.pos];
        self.pos += 1;
        Ok(b)
    }

    /// Peek at `bits` bits from the MSB-first bitstream without consuming them.
    fn look_bits(&mut self, bits: i32) -> io::Result<u32> {
        while self.valid < bits {
            let b = self.read_byte()? as u64;
            self.bitbuf = (self.bitbuf << 8) | b;
            self.valid += 8;
        }
        let mask = (1u32 << bits) - 1;
        Ok(((self.bitbuf >> (self.valid - bits)) as u32) & mask)
    }

    /// Skip `bits` bits (after having peeked at them).
    fn skip_bits(&mut self, bits: i32) {
        self.valid -= bits;
    }
}

/// Decompress data in GNU `pack` format.
///
/// `data` is everything AFTER the 2-byte magic (`1f 1e`).
pub fn decompress_pack<W: Write>(data: &[u8], writer: &mut W) -> io::Result<()> {
    let mut reader = BitReader::new(data);

    // --- Read the Huffman tree description ---

    // 4 bytes: original uncompressed length, big-endian
    let mut orig_len: u32 = 0;
    for _ in 0..4 {
        orig_len = (orig_len << 8) | reader.read_byte()? as u32;
    }

    // 1 byte: maximum bit length
    let max_len = reader.read_byte()? as usize;
    if max_len == 0 || max_len > MAX_BITLEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid compressed data -- Huffman code bit length out of range",
        ));
    }

    // Read the number of leaves at each bit length
    let mut leaves = [0i32; MAX_BITLEN + 1];
    let mut max_leaves: i32 = 1;
    let mut n = 0i32;
    #[allow(clippy::needless_range_loop)]
    for len in 1..=max_len {
        leaves[len] = reader.read_byte()? as i32;
        if max_leaves - (if len == max_len { 1 } else { 0 }) < leaves[len] {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "too many leaves in Huffman tree",
            ));
        }
        max_leaves = (max_leaves - leaves[len] + 1) * 2 - 1;
        n += leaves[len];
    }
    if n >= LITERALS as i32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "too many leaves in Huffman tree",
        ));
    }

    // The last leaf count is biased: the EOB code is implicit.
    // Add 1 to include it in the tree.
    leaves[max_len] += 1;

    // Read the literal byte values
    let mut literal = [0u8; LITERALS];
    let mut lit_base = [0i32; MAX_BITLEN + 1];
    let mut base = 0usize;
    #[allow(clippy::needless_range_loop)]
    for len in 1..=max_len {
        lit_base[len] = base as i32;
        for _ in 0..leaves[len] {
            if base >= LITERALS {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "too many leaves in Huffman tree",
                ));
            }
            literal[base] = reader.read_byte()?;
            base += 1;
        }
    }

    // Now include the EOB code in the tree structure
    leaves[max_len] += 1;

    // --- Build the prefix table ---

    let mut parents = [0i32; MAX_BITLEN + 1];
    let mut nodes: i32 = 0;
    for len in (1..=max_len).rev() {
        nodes >>= 1;
        parents[len] = nodes;
        lit_base[len] -= nodes;
        nodes += leaves[len];
    }
    if (nodes >> 1) != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "too few leaves in Huffman tree",
        ));
    }

    let peek_bits = max_len.min(MAX_PEEK);
    let table_size = 1usize << peek_bits;
    let mut prefix_len = vec![0u8; table_size];

    // Fill from shortest codes to longest, working backwards in the table.
    // The shortest code is all ones, so we start at the end.
    let mut idx = table_size;
    #[allow(clippy::needless_range_loop)]
    for len in 1..=peek_bits {
        let prefixes = (leaves[len] as usize) << (peek_bits - len);
        for _ in 0..prefixes {
            idx -= 1;
            prefix_len[idx] = len as u8;
        }
    }
    // Remaining entries (0..idx) stay 0 — codes longer than peek_bits.

    // --- Decode the bitstream ---

    // The EOB code is the largest code among all leaves of max_len
    let eob = (leaves[max_len] - 1) as u32;
    let peek_mask = (1u32 << peek_bits) - 1;

    let mut bytes_out: u64 = 0;
    let mut outbuf = Vec::with_capacity(16384);

    loop {
        // Peek at peek_bits bits
        let mut peek = reader.look_bits(peek_bits as i32)?;
        let len;

        let plen = prefix_len[peek as usize];
        if plen > 0 {
            len = plen as i32;
            peek >>= peek_bits as u32 - len as u32;
        } else {
            // Code longer than peek_bits — traverse the tree
            let mut mask = peek_mask;
            let mut l = peek_bits as i32;
            while (peek as i32) < parents[l as usize] {
                l += 1;
                mask = (mask << 1) + 1;
                peek = reader.look_bits(l)?;
            }
            len = l;
        }

        // Check for EOB
        if peek == eob && len == max_len as i32 {
            break;
        }

        // Output the literal byte
        let idx = (peek as i32 + lit_base[len as usize]) as usize;
        if idx >= LITERALS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid compressed data--format violated",
            ));
        }
        outbuf.push(literal[idx]);
        bytes_out += 1;

        if outbuf.len() >= 16384 {
            writer.write_all(&outbuf)?;
            outbuf.clear();
        }

        reader.skip_bits(len);
    }

    // Flush remaining output
    if !outbuf.is_empty() {
        writer.write_all(&outbuf)?;
    }
    writer.flush()?;

    // Verify length
    if orig_len as u64 != (bytes_out & 0xFFFF_FFFF) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid compressed data--length error",
        ));
    }

    Ok(())
}
