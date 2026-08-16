//! Bounds-checked big-endian byte reader used by all box parsing.
//!
//! Every read returns `Err(HeifError::Truncated)` instead of panicking, so
//! arbitrary untrusted bytes can be fed to the parser safely. The reader
//! carries a `context` label naming the structure being parsed; it appears in
//! truncation errors to make corrupt-file reports actionable.

use crate::error::HeifError;

/// A cursor over a byte slice with big-endian primitive reads.
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    /// Name of the structure being parsed, used in error messages.
    context: &'static str,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8], context: &'static str) -> Self {
        Reader {
            data,
            pos: 0,
            context,
        }
    }

    /// Bytes remaining after the cursor.
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    /// Current cursor position from the start of the slice.
    pub fn pos(&self) -> usize {
        self.pos
    }

    fn truncated(&self) -> HeifError {
        HeifError::Truncated(self.context)
    }

    /// Take `n` raw bytes.
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], HeifError> {
        if self.remaining() < n {
            return Err(self.truncated());
        }
        let out = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    /// Skip `n` bytes.
    pub fn skip(&mut self, n: usize) -> Result<(), HeifError> {
        self.bytes(n).map(|_| ())
    }

    pub fn u8(&mut self) -> Result<u8, HeifError> {
        Ok(self.bytes(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16, HeifError> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> Result<u32, HeifError> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn u64(&mut self) -> Result<u64, HeifError> {
        let b = self.bytes(8)?;
        Ok(u64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    /// Read an unsigned integer of 0, 4 or 8 bytes — the variable-width
    /// offset/length fields `iloc` declares via its `*_size` header fields.
    /// A width of 0 is defined by ISOBMFF to mean the value 0.
    pub fn uint_sized(&mut self, byte_width: u8) -> Result<u64, HeifError> {
        match byte_width {
            0 => Ok(0),
            4 => Ok(self.u32()? as u64),
            8 => self.u64(),
            // iloc technically allows any of {0,4,8}; other widths are
            // structurally invalid per ISO 14496-12 §8.11.3.2.
            _ => Err(HeifError::Invalid(format!(
                "invalid field width {} in {}",
                byte_width, self.context
            ))),
        }
    }

    /// Read a fourcc as a `[u8; 4]`.
    pub fn fourcc(&mut self) -> Result<[u8; 4], HeifError> {
        let b = self.bytes(4)?;
        Ok([b[0], b[1], b[2], b[3]])
    }

    /// Read a full-box header: 1-byte version + 3-byte flags.
    pub fn full_box_header(&mut self) -> Result<(u8, u32), HeifError> {
        let version = self.u8()?;
        let b = self.bytes(3)?;
        let flags = u32::from_be_bytes([0, b[0], b[1], b[2]]);
        Ok((version, flags))
    }

    /// Read a NUL-terminated UTF-8 string (used by `infe` item names).
    /// Consumes through the terminator; unterminated data consumes to the end.
    pub fn c_string(&mut self) -> Result<&'a str, HeifError> {
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos] != 0 {
            self.pos += 1;
        }
        let s = std::str::from_utf8(&self.data[start..self.pos]).unwrap_or("");
        if self.pos < self.data.len() {
            self.pos += 1; // consume the NUL
        }
        Ok(s)
    }
}

/// Render a fourcc for error messages, escaping non-printable bytes.
pub fn fourcc_str(fourcc: [u8; 4]) -> String {
    fourcc
        .iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                (b as char).to_string()
            } else {
                format!("\\x{b:02x}")
            }
        })
        .collect()
}
