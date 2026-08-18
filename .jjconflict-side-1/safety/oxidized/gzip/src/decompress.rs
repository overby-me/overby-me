use flate2::{Crc, Decompress, FlushDecompress, Status};
use std::io::{self, Read, Write};

use crate::unlzw;
use crate::unpack;

/// Decode a full stream, supporting multi-member gzip, trailing NUL
/// padding (tape archive convention), `-f`-style cat pass-through
/// for non-gzip content, and legacy pack/compress formats.
///
/// We buffer the whole input so we can walk member boundaries exactly —
/// flate2’s streaming decoders over-read into their own buffers and
/// would lose bytes belonging to the tail.
pub fn decompress_stream<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    force: bool,
) -> io::Result<()> {
    let mut input = Vec::new();
    reader.read_to_end(&mut input)?;
    let mut pos = 0;
    let mut any_member = false;
    while pos < input.len() {
        let remaining = &input[pos..];
        if remaining.len() >= 2 && remaining[0] == 0x1f && remaining[1] == 0x8b {
            // Gzip member
            let consumed = decode_gzip_member(remaining, &mut writer)?;
            pos += consumed;
            any_member = true;
        } else if remaining.len() >= 2 && remaining[0] == 0x1f && remaining[1] == 0x1e {
            // Pack format (magic 1f 1e) — single-member, consume all
            unpack::decompress_pack(&remaining[2..], &mut writer)?;
            pos = input.len();
            any_member = true;
        } else if remaining.len() >= 3 && remaining[0] == 0x1f && remaining[1] == 0x9d {
            // LZW / Unix compress format (magic 1f 9d) — single-member, consume all
            unlzw::decompress_lzw(&remaining[2..], &mut writer)?;
            pos = input.len();
            any_member = true;
        } else if any_member && remaining.iter().all(|&b| b == 0) {
            // Trailing NUL padding after at least one valid member is
            // silently tolerated (tape alignment, etc.).
            break;
        } else if force {
            writer.write_all(remaining)?;
            pos = input.len();
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not in gzip format",
            ));
        }
    }
    writer.flush()?;
    Ok(())
}

/// Decode one gzip member from `data` and return the number of bytes
/// consumed (header + deflate body + 8-byte trailer).
fn decode_gzip_member<W: Write>(data: &[u8], writer: &mut W) -> io::Result<usize> {
    let header_len = parse_gzip_header(data)?;
    let mut body_pos = header_len;
    let mut decomp = Decompress::new(false);
    let mut crc = Crc::new();
    let mut out_buf = vec![0u8; 65536];
    loop {
        let in_before = decomp.total_in();
        let out_before = decomp.total_out();
        let status = decomp
            .decompress(&data[body_pos..], &mut out_buf, FlushDecompress::None)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let consumed = (decomp.total_in() - in_before) as usize;
        let produced = (decomp.total_out() - out_before) as usize;
        body_pos += consumed;
        if produced > 0 {
            crc.update(&out_buf[..produced]);
            writer.write_all(&out_buf[..produced])?;
        }
        if matches!(status, Status::StreamEnd) {
            break;
        }
        if consumed == 0 && produced == 0 {
            // The deflate decompressor consumed all available input
            // without reaching StreamEnd. GNU gzip's own inflate.c
            // would detect structural errors in the Huffman tables
            // and return "format violated"; zlib just says "need more
            // data". Match GNU gzip's behavior.
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid compressed data--format violated",
            ));
        }
    }
    // 8-byte trailer: CRC32 + ISIZE, both little-endian.
    if body_pos + 8 > data.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "unexpected end of file",
        ));
    }
    let expected_crc = u32::from_le_bytes([
        data[body_pos],
        data[body_pos + 1],
        data[body_pos + 2],
        data[body_pos + 3],
    ]);
    let expected_isize = u32::from_le_bytes([
        data[body_pos + 4],
        data[body_pos + 5],
        data[body_pos + 6],
        data[body_pos + 7],
    ]);
    if crc.sum() != expected_crc {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid compressed data--crc error",
        ));
    }
    let actual_isize = (decomp.total_out() & 0xFFFF_FFFF) as u32;
    if actual_isize != expected_isize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid compressed data--length error",
        ));
    }
    Ok(body_pos + 8)
}

/// Parse the gzip member header. Returns the header length on success.
fn parse_gzip_header(data: &[u8]) -> io::Result<usize> {
    // 10-byte fixed header: magic(2) + method(1) + flags(1) + mtime(4)
    //                      + xfl(1) + os(1).
    if data.len() < 10 || data[0] != 0x1f || data[1] != 0x8b {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not in gzip format",
        ));
    }
    if data[2] != 8 {
        // Only deflate (method 8) is defined.
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not in gzip format",
        ));
    }
    let flags = data[3];
    let mut p = 10;
    if flags & 0x04 != 0 {
        // FEXTRA: 2-byte length, then that many bytes.
        if data.len() < p + 2 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected end of file",
            ));
        }
        let xlen = u16::from_le_bytes([data[p], data[p + 1]]) as usize;
        p += 2 + xlen;
    }
    if flags & 0x08 != 0 {
        // FNAME: NUL-terminated original filename.
        while p < data.len() && data[p] != 0 {
            p += 1;
        }
        if p >= data.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected end of file",
            ));
        }
        p += 1;
    }
    if flags & 0x10 != 0 {
        // FCOMMENT: NUL-terminated comment.
        while p < data.len() && data[p] != 0 {
            p += 1;
        }
        if p >= data.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected end of file",
            ));
        }
        p += 1;
    }
    if flags & 0x02 != 0 {
        // FHCRC: 2-byte header CRC.
        p += 2;
        if p > data.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected end of file",
            ));
        }
    }
    Ok(p)
}

/// Map flate2’s deflate/gzip error strings to GNU gzip’s canonical wording
/// so upstream tests (hufts, helin-segv, trailing-nul, ...) can compare
/// stderr byte-for-byte.
pub fn canonical_decode_error(e: &io::Error) -> String {
    let s = e.to_string();
    let l = s.to_ascii_lowercase();
    if l.contains("unexpected eof") || l.contains("unexpected end of file") {
        "unexpected end of file".to_string()
    } else if l.contains("invalid gzip header")
        || l.contains("not in gzip")
        || l.contains("invalid magic")
    {
        "not in gzip format".to_string()
    } else if l.contains("corrupt")
        || l.contains("invalid block")
        || l.contains("invalid distance")
        || l.contains("invalid literal")
        || l.contains("invalid deflate")
        || l.contains("format violated")
        || l.contains("deflate decompression")
        || l.contains("decompress")
    {
        "invalid compressed data--format violated".to_string()
    } else if l.contains("crc") {
        "invalid compressed data--crc error".to_string()
    } else if l.contains("length error") {
        "invalid compressed data--length error".to_string()
    } else {
        s
    }
}
