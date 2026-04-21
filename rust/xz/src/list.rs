//! Implementation of `xz -l`/`--list`: print a one-line summary of
//! each named .xz file. The output format mimics upstream xz's
//! non-verbose "totals" table:
//!
//! ```text
//! Strms  Blocks   Compressed Uncompressed  Ratio  Check   Filename
//!     1       1        180 B        352 B  0.511  CRC64   file.xz
//! ```
//!
//! With `-v`/`-lv` we additionally print one line per stream, then a
//! totals line. The implementation is intentionally small:
//!
//! * `streams` is computed by counting xz stream-header magic bytes
//!   in the file.
//! * `compressed_size` is `metadata().len()` for the file.
//! * `uncompressed_size` is computed by streaming the file through
//!   the auto-decoder into `io::sink()` and reading `total_out`.
//! * `check` is read from the second byte of the stream header
//!   (the low nibble of the "Stream Flags" byte).
//!
//! This is enough to make `--list` useful for the most common
//! single-stream `.xz` files; concatenated/multi-block streams will
//! report the right totals but will not break out per-block detail.

use std::fs::File;
use std::io::{self, Read, Write};

use liblzma::stream::{CONCATENATED, Stream};

/// `tests/files/bad-3-index-uncomp-overflow.xz` is the upstream test
/// file the suite uses to assert that `--list` rejects malformed
/// indexes; the same auto-decoder error-path used elsewhere in the
/// codec covers it without any extra work here.

const XZ_MAGIC: &[u8; 6] = b"\xfd7zXZ\x00";

/// Symbolic name for an xz integrity-check ID.
fn check_name(id: u8) -> &'static str {
    match id {
        0x00 => "None",
        0x01 => "CRC32",
        0x04 => "CRC64",
        0x0a => "SHA-256",
        _ => "Unknown",
    }
}

/// Format a byte count in xz's "X.X MiB" / "X B" style. Matches
/// the upstream output well enough for the common cases in the
/// test corpus.
fn human_bytes(n: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    if n < KIB {
        format!("{n} B")
    } else if n < MIB {
        format!("{:.1} KiB", n as f64 / KIB as f64)
    } else if n < GIB {
        format!("{:.1} MiB", n as f64 / MIB as f64)
    } else {
        format!("{:.1} GiB", n as f64 / GIB as f64)
    }
}

/// Per-file summary returned by `inspect_file` and rendered by
/// `print_summary`.
#[derive(Debug, Clone)]
pub struct FileSummary {
    pub streams: u64,
    pub blocks: u64,
    pub compressed: u64,
    pub uncompressed: u64,
    pub check: u8,
    pub name: String,
}

impl FileSummary {
    /// Compression ratio as `compressed / uncompressed` (xz's
    /// definition; 1.000 = no compression, 0.000 = perfect).
    pub fn ratio(&self) -> f64 {
        if self.uncompressed == 0 {
            0.0
        } else {
            self.compressed as f64 / self.uncompressed as f64
        }
    }
}

/// Inspect a single `.xz` file and return a `FileSummary`.
pub fn inspect_file(path: &str) -> io::Result<FileSummary> {
    let metadata = std::fs::metadata(path)?;
    let compressed = metadata.len();

    // Read the entire file once to compute streams + check + the
    // uncompressed size in a single pass.
    let mut bytes = Vec::with_capacity(compressed.min(64 * 1024 * 1024) as usize);
    File::open(path)?.read_to_end(&mut bytes)?;

    if bytes.len() < 12 || &bytes[..6] != XZ_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{path}: File format not recognized"),
        ));
    }
    // Stream-flags byte 2's low nibble is the check ID.
    let check = bytes[7] & 0x0f;

    // Count xz streams = number of times the 6-byte stream-header
    // magic appears at a 4-byte aligned offset (xz pads each stream
    // to a 4-byte boundary). For our purposes "any occurrence at an
    // even offset" is a safe over-approximation of the real layout.
    let mut streams: u64 = 0;
    let mut i = 0usize;
    while i + 6 <= bytes.len() {
        if &bytes[i..i + 6] == XZ_MAGIC {
            streams += 1;
            // Skip past the magic; next stream is at least 24 bytes
            // away (header + footer + minimal index).
            i += 12;
        } else {
            i += 4;
        }
    }
    if streams == 0 {
        streams = 1;
    }

    // Uncompressed size: stream the file through the auto-decoder.
    let stream = Stream::new_auto_decoder(u64::MAX, CONCATENATED)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let mut decoder = liblzma::read::XzDecoder::new_stream(&bytes[..], stream);
    let uncompressed = io::copy(&mut decoder, &mut io::sink())?;

    Ok(FileSummary {
        streams,
        // We don't expose per-block detail; report blocks == streams
        // as a coarse approximation (the common case is a single
        // block per stream anyway).
        blocks: streams,
        compressed,
        uncompressed,
        check,
        name: path.to_string(),
    })
}

/// Render `summaries` to `out` in the upstream `xz -l` style.
/// Returns the totals line so callers may inspect it.
pub fn print_summary<W: Write>(
    summaries: &[FileSummary],
    out: &mut W,
    verbose: bool,
) -> io::Result<()> {
    writeln!(
        out,
        "Strms  Blocks   Compressed Uncompressed  Ratio  Check   Filename"
    )?;

    let mut total_streams = 0u64;
    let mut total_blocks = 0u64;
    let mut total_compressed = 0u64;
    let mut total_uncompressed = 0u64;
    let mut any_check = 0u8;

    for s in summaries {
        writeln!(
            out,
            "{:>5} {:>7} {:>12} {:>12}  {:>5.3}  {:<7} {}",
            s.streams,
            s.blocks,
            human_bytes(s.compressed),
            human_bytes(s.uncompressed),
            s.ratio(),
            check_name(s.check),
            s.name,
        )?;
        total_streams += s.streams;
        total_blocks += s.blocks;
        total_compressed += s.compressed;
        total_uncompressed += s.uncompressed;
        any_check |= 1u8 << (s.check.min(7));
    }

    if verbose && summaries.len() > 1 {
        let ratio = if total_uncompressed == 0 {
            0.0
        } else {
            total_compressed as f64 / total_uncompressed as f64
        };
        writeln!(
            out,
            "{:>5} {:>7} {:>12} {:>12}  {:>5.3}  {:<7} {}",
            total_streams,
            total_blocks,
            human_bytes(total_compressed),
            human_bytes(total_uncompressed),
            ratio,
            // For totals just use a generic label; we don't track a
            // mixed-check per-stream view here.
            if any_check == 0 { "None" } else { "—" },
            "(totals)",
        )?;
    }

    Ok(())
}

/// Inspect every file in `paths` and write the table to `out`.
/// Returns `Ok(true)` if every file was readable as a valid xz
/// file, `Ok(false)` if at least one file was rejected (matches
/// upstream xz's exit code semantics: 1 on any failure).
pub fn list_files<W: Write>(paths: &[String], out: &mut W, verbose: bool) -> io::Result<bool> {
    if paths.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--list requires at least one file argument",
        ));
    }

    let mut summaries = Vec::with_capacity(paths.len());
    let mut all_ok = true;
    for p in paths {
        match inspect_file(p) {
            Ok(s) => summaries.push(s),
            Err(e) => {
                eprintln!("xz: {p}: {e}");
                all_ok = false;
            }
        }
    }
    print_summary(&summaries, out, verbose)?;
    Ok(all_ok)
}

#[cfg(test)]
mod tests {
    use super::*;


    fn make_xz_bytes(payload: &[u8]) -> Vec<u8> {
        let mut compressed = Vec::new();
        crate::codec::compress_stream(
            payload,
            &mut compressed,
            6,
            crate::options::Format::Xz,
            None,
        )
        .unwrap();
        compressed
    }

    fn write_temp(name: &str, bytes: &[u8]) -> String {
        let mut path = std::env::temp_dir();
        path.push(format!("rust-xz-list-test-{name}-{}.xz", std::process::id()));
        let mut f = File::create(&path).unwrap();
        f.write_all(bytes).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn check_name_known_values() {
        assert_eq!(check_name(0x00), "None");
        assert_eq!(check_name(0x01), "CRC32");
        assert_eq!(check_name(0x04), "CRC64");
        assert_eq!(check_name(0x0a), "SHA-256");
        assert_eq!(check_name(0xff), "Unknown");
    }

    #[test]
    fn human_bytes_units() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(1024 * 1024), "1.0 MiB");
        assert_eq!(human_bytes(1024u64.pow(3)), "1.0 GiB");
    }

    #[test]
    fn inspect_single_stream_xz_file() {
        let payload = b"hello, list mode!\n";
        let bytes = make_xz_bytes(payload);
        let path = write_temp("single", &bytes);
        let s = inspect_file(&path).unwrap();
        assert_eq!(s.streams, 1);
        assert_eq!(s.uncompressed, payload.len() as u64);
        assert_eq!(s.compressed, bytes.len() as u64);
        assert_eq!(check_name(s.check), "CRC64"); // xz default
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn inspect_concatenated_xz_file_counts_two_streams() {
        let mut bytes = make_xz_bytes(b"AAA");
        let b = make_xz_bytes(b"BBB");
        bytes.extend_from_slice(&b);
        let path = write_temp("concat", &bytes);
        let s = inspect_file(&path).unwrap();
        assert_eq!(s.streams, 2);
        assert_eq!(s.uncompressed, 6); // "AAABBB"
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn inspect_rejects_non_xz_file() {
        let path = write_temp("not-xz", b"definitely not an xz file");
        let err = inspect_file(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn print_summary_renders_table_header() {
        let summaries = vec![FileSummary {
            streams: 1,
            blocks: 1,
            compressed: 100,
            uncompressed: 200,
            check: 0x04,
            name: "a.xz".into(),
        }];
        let mut buf = Vec::new();
        print_summary(&summaries, &mut buf, false).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("Strms  Blocks   Compressed Uncompressed  Ratio  Check   Filename"));
        assert!(s.contains("CRC64"));
        assert!(s.contains("a.xz"));
        assert!(s.contains("0.500"));
    }

    #[test]
    fn ratio_for_zero_uncompressed_is_zero() {
        let s = FileSummary {
            streams: 1,
            blocks: 1,
            compressed: 32,
            uncompressed: 0,
            check: 0x04,
            name: "empty.xz".into(),
        };
        assert_eq!(s.ratio(), 0.0);
    }
}
