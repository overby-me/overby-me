//! Shared configuration for ZIP entry decompression limits.
//!
//! OOXML parsers must cap per-entry decompressed output to block zip-bomb DoS.
//! The cap defaults to 512 MiB — large enough for legitimate embedded video /
//! 4K media but small enough to refuse pathological archives — and can be
//! overridden per-parse via the `wasm_bindgen` entry points so library users
//! can tighten the budget (untrusted gateways) or loosen it (legitimate huge
//! decks) without forking.
//!
//! The current cap is stored in a thread-local so existing `read_zip_*`
//! helpers in each parser can consult it without threading a parameter
//! through ~70 call sites. Each WASM entry point installs a [`Guard`] for
//! its scope and the cap is restored on drop, so concurrent JS callers never
//! interfere (WASM is single-threaded; each invocation runs to completion).

use std::cell::Cell;

/// 512 MiB. OOXML legitimately reaches tens of MB (embedded video, 4K
/// images) but not hundreds, so this cap blocks zip-bomb DoS without
/// rejecting real files.
pub const DEFAULT_MAX_ZIP_ENTRY_BYTES: u64 = 512 * 1024 * 1024;

/// Upper bound on the buffer we pre-reserve from an entry's DECLARED size.
///
/// `entry.size()` is the uncompressed size recorded in the zip header, which is
/// attacker-controlled: a zip-bomb variant can declare 512 MiB (up to the entry
/// cap) while the real payload is a few bytes. Feeding that straight into
/// `Vec::with_capacity` wastes up to `DEFAULT_MAX_ZIP_ENTRY_BYTES` of eager
/// allocation per entry. We instead pre-reserve at most 1 MiB and let
/// `read_to_end` grow the buffer for genuinely large parts.
///
/// 1 MiB is chosen because it comfortably fits the vast majority of OOXML parts
/// (document.xml / sheetN.xml / slideN.xml are typically tens to a few hundred
/// KiB) so they incur zero reallocation, while capping the wasted reserve for a
/// forged header at 1 MiB. Genuinely large parts (multi-MB sharedStrings) grow
/// via `read_to_end`'s amortized-O(n) doubling — a handful of reallocs, no
/// measurable parse-time cost (verified against the parse bench).
const INITIAL_RESERVE_CAP: usize = 1024 * 1024; // 1 MiB

/// Buffer capacity to pre-reserve for an entry that declares `declared_size`
/// uncompressed bytes: the declared size when small, else [`INITIAL_RESERVE_CAP`].
/// Clamps the eager `with_capacity` so a forged declaration cannot force a giant
/// up-front allocation; the read still completes in full via `read_to_end`.
fn initial_reserve(declared_size: u64) -> usize {
    declared_size.min(INITIAL_RESERVE_CAP as u64) as usize
}

thread_local! {
    static MAX_ZIP_ENTRY_BYTES: Cell<u64> = const { Cell::new(DEFAULT_MAX_ZIP_ENTRY_BYTES) };
}

/// RAII guard that restores the previous cap when dropped. Created by
/// [`scoped_max`]; the caller should bind it to a `let _guard = …` for the
/// full duration of the parse call.
#[must_use = "binding the guard keeps the cap installed for this scope"]
pub struct Guard {
    previous: u64,
}

impl Drop for Guard {
    fn drop(&mut self) {
        MAX_ZIP_ENTRY_BYTES.with(|c| c.set(self.previous));
    }
}

/// Install a per-call ZIP entry size cap for the lifetime of the returned
/// guard. `None`, zero, or any non-positive value falls back to
/// [`DEFAULT_MAX_ZIP_ENTRY_BYTES`].
pub fn scoped_max(value: Option<u64>) -> Guard {
    let resolved = value
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_MAX_ZIP_ENTRY_BYTES);
    let previous = MAX_ZIP_ENTRY_BYTES.with(|c| c.replace(resolved));
    Guard { previous }
}

/// Current cap in effect on this thread. Parsers consult this from their
/// `read_zip_*` helpers when validating entry sizes.
pub fn current_max() -> u64 {
    MAX_ZIP_ENTRY_BYTES.with(Cell::get)
}

/// Read one zip entry's bytes by path. Honors the scoped max-entry guard:
/// entries whose declared size exceeds the cap (default 512 MiB, or the
/// per-call override) are rejected rather than truncated — the zip-bomb DoS
/// guard shared with the per-parser `extract_*` WASM entry points.
pub fn extract_zip_entry(
    data: &[u8],
    path: &str,
    max_zip_entry_bytes: Option<u64>,
) -> Result<Vec<u8>, String> {
    use std::io::{Cursor, Read};
    let _guard = scoped_max(max_zip_entry_bytes);
    let max = current_max();
    let cursor = Cursor::new(data);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| format!("zip open error: {e}"))?;
    let mut entry = zip
        .by_name(path)
        .map_err(|e| format!("entry not found: {path}: {e}"))?;
    if entry.size() > max {
        return Err(format!("ZIP entry exceeds size limit: {path}"));
    }
    // Pre-reserve a capped amount, not the (attacker-controlled) declared size —
    // `read_to_end` grows the buffer for genuinely large parts. See INITIAL_RESERVE_CAP.
    let mut buf = Vec::with_capacity(initial_reserve(entry.size()));
    entry
        .by_ref()
        .take(max)
        .read_to_end(&mut buf)
        .map_err(|e| format!("read error: {e}"))?;
    Ok(buf)
}

/// Read one entry's bytes from an **already-opened** [`ZipArchive`]. Twin of
/// [`extract_zip_entry`] for callers that keep a single archive open across
/// many reads (the common case inside a parser) instead of re-opening it from
/// the raw bytes per entry. Honors the scoped max-entry guard: an entry whose
/// declared size exceeds the current cap is rejected with an `Err`, never
/// silently truncated (the zip-bomb DoS guard). Generic over the archive's
/// reader so each parser's concrete type (`Cursor<&[u8]>`, …) works unchanged.
pub fn read_zip_bytes<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    path: &str,
) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let max = current_max();
    let mut entry = archive
        .by_name(path)
        .map_err(|e| format!("entry not found: {path}: {e}"))?;
    if entry.size() > max {
        return Err(format!("ZIP entry exceeds size limit: {path}"));
    }
    // Pre-reserve a capped amount, not the (attacker-controlled) declared size —
    // `read_to_end` grows the buffer for genuinely large parts. See INITIAL_RESERVE_CAP.
    let mut buf = Vec::with_capacity(initial_reserve(entry.size()));
    entry
        .by_ref()
        .take(max)
        .read_to_end(&mut buf)
        .map_err(|e| format!("read error: {e}"))?;
    Ok(buf)
}

/// UTF-8 string counterpart of [`read_zip_bytes`] for XML parts. Same cap
/// enforcement and archive-reuse contract; decodes the entry as UTF-8 (strict —
/// OOXML parts are well-formed UTF-8, and a decode failure is a real corruption
/// worth reporting rather than papering over with lossy substitution).
pub fn read_zip_string<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    path: &str,
) -> Result<String, String> {
    use std::io::Read;
    let max = current_max();
    let mut entry = archive
        .by_name(path)
        .map_err(|e| format!("entry not found: {path}: {e}"))?;
    if entry.size() > max {
        return Err(format!("ZIP entry exceeds size limit: {path}"));
    }
    let mut buf = String::new();
    entry
        .by_ref()
        .take(max)
        .read_to_string(&mut buf)
        .map_err(|e| format!("read error: {e}"))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_zip_entry_reads_by_path() {
        use std::io::{Cursor, Write};
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default();
            w.start_file("ppt/media/image1.png", opts).unwrap();
            w.write_all(b"\x89PNGdata").unwrap();
            w.finish().unwrap();
        }
        let bytes = extract_zip_entry(&buf, "ppt/media/image1.png", None).unwrap();
        assert_eq!(bytes, b"\x89PNGdata");
        assert!(extract_zip_entry(&buf, "ppt/media/missing.png", None)
            .unwrap_err()
            .contains("not found"));
    }

    #[test]
    fn extract_zip_entry_rejects_oversized_entry() {
        use std::io::{Cursor, Write};
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default();
            w.start_file("ppt/media/big.bin", opts).unwrap();
            w.write_all(b"12345678").unwrap(); // 8 bytes uncompressed
            w.finish().unwrap();
        }
        // A cap below the declared size must be REJECTED, never silently
        // truncated — this is the zip-bomb DoS guard (default 512 MiB).
        let err = extract_zip_entry(&buf, "ppt/media/big.bin", Some(4)).unwrap_err();
        assert!(err.contains("exceeds size limit"), "got: {err}");
        // A cap above the size reads the entry in full.
        assert_eq!(
            extract_zip_entry(&buf, "ppt/media/big.bin", Some(64)).unwrap(),
            b"12345678"
        );
    }

    /// Build a one-entry in-memory zip for the open-archive helper tests.
    fn archive_with(name: &str, body: &[u8]) -> zip::ZipArchive<std::io::Cursor<Vec<u8>>> {
        use std::io::{Cursor, Write};
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default();
            w.start_file(name, opts).unwrap();
            w.write_all(body).unwrap();
            w.finish().unwrap();
        }
        zip::ZipArchive::new(Cursor::new(buf)).unwrap()
    }

    #[test]
    fn read_zip_bytes_reads_present_and_reports_missing() {
        let mut ar = archive_with("word/document.xml", b"<xml/>");
        assert_eq!(
            read_zip_bytes(&mut ar, "word/document.xml").unwrap(),
            b"<xml/>"
        );
        let err = read_zip_bytes(&mut ar, "word/missing.xml").unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[test]
    fn read_zip_string_reads_present_and_reports_missing() {
        let mut ar = archive_with("xl/workbook.xml", b"<workbook/>");
        assert_eq!(
            read_zip_string(&mut ar, "xl/workbook.xml").unwrap(),
            "<workbook/>"
        );
        let err = read_zip_string(&mut ar, "xl/nope.xml").unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }

    /// Forge a STORED (compression=0) single-entry zip whose declared
    /// `uncompressed_size` (in BOTH the local file header and the central
    /// directory) is much larger than the real body. Returns the raw zip bytes.
    ///
    /// A stored entry lets us set uncompressed==compressed==declared cleanly.
    /// We hand-lay the bytes so we control the size fields the way a malicious
    /// archive would. `real` is the actual payload; `declared` is the lie.
    #[cfg(test)]
    fn forged_stored_zip(name: &str, real: &[u8], declared: u32) -> Vec<u8> {
        let crc = {
            // CRC-32 of the real body (zip stores the checksum of actual data).
            const POLY: u32 = 0xEDB8_8320;
            let mut c: u32 = 0xFFFF_FFFF;
            for &b in real {
                c ^= b as u32;
                for _ in 0..8 {
                    c = if c & 1 != 0 { (c >> 1) ^ POLY } else { c >> 1 };
                }
            }
            !c
        };
        let name_bytes = name.as_bytes();
        let nlen = name_bytes.len() as u16;
        let mut z = Vec::new();
        // ── Local file header ──
        z.extend_from_slice(&0x0403_4b50u32.to_le_bytes()); // signature PK\x03\x04
        z.extend_from_slice(&20u16.to_le_bytes()); // version needed
        z.extend_from_slice(&0u16.to_le_bytes()); // flags
        z.extend_from_slice(&0u16.to_le_bytes()); // method = 0 (stored)
        z.extend_from_slice(&0u16.to_le_bytes()); // mod time
        z.extend_from_slice(&0u16.to_le_bytes()); // mod date
        z.extend_from_slice(&crc.to_le_bytes()); // crc-32
        z.extend_from_slice(&declared.to_le_bytes()); // compressed size (LIE)
        z.extend_from_slice(&declared.to_le_bytes()); // uncompressed size (LIE)
        z.extend_from_slice(&nlen.to_le_bytes()); // file name length
        z.extend_from_slice(&0u16.to_le_bytes()); // extra length
        z.extend_from_slice(name_bytes);
        let data_start = z.len();
        z.extend_from_slice(real); // only `real.len()` bytes actually present
                                   // ── Central directory header ──
        let cd_start = z.len();
        z.extend_from_slice(&0x0201_4b50u32.to_le_bytes()); // signature PK\x01\x02
        z.extend_from_slice(&20u16.to_le_bytes()); // version made by
        z.extend_from_slice(&20u16.to_le_bytes()); // version needed
        z.extend_from_slice(&0u16.to_le_bytes()); // flags
        z.extend_from_slice(&0u16.to_le_bytes()); // method = 0 (stored)
        z.extend_from_slice(&0u16.to_le_bytes()); // mod time
        z.extend_from_slice(&0u16.to_le_bytes()); // mod date
        z.extend_from_slice(&crc.to_le_bytes()); // crc-32
        z.extend_from_slice(&declared.to_le_bytes()); // compressed size (LIE)
        z.extend_from_slice(&declared.to_le_bytes()); // uncompressed size (LIE)
        z.extend_from_slice(&nlen.to_le_bytes()); // file name length
        z.extend_from_slice(&0u16.to_le_bytes()); // extra length
        z.extend_from_slice(&0u16.to_le_bytes()); // comment length
        z.extend_from_slice(&0u16.to_le_bytes()); // disk number start
        z.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        z.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        z.extend_from_slice(&(data_start as u32 - 30 - nlen as u32).to_le_bytes()); // local header offset (=0)
        z.extend_from_slice(name_bytes);
        let cd_size = z.len() - cd_start;
        // ── End of central directory ──
        z.extend_from_slice(&0x0605_4b50u32.to_le_bytes()); // signature PK\x05\x06
        z.extend_from_slice(&0u16.to_le_bytes()); // disk number
        z.extend_from_slice(&0u16.to_le_bytes()); // cd start disk
        z.extend_from_slice(&1u16.to_le_bytes()); // entries on this disk
        z.extend_from_slice(&1u16.to_le_bytes()); // total entries
        z.extend_from_slice(&(cd_size as u32).to_le_bytes()); // cd size
        z.extend_from_slice(&(cd_start as u32).to_le_bytes()); // cd offset
        z.extend_from_slice(&0u16.to_le_bytes()); // comment length
        z
    }

    /// EMPIRICAL (RB11 attack-vector confirmation): `entry.size()` reports the
    /// DECLARED (attacker-controlled, central-directory) uncompressed size, NOT
    /// the actual decompressed byte count. This is the number the pre-fix code
    /// fed straight into `Vec::with_capacity`, so a forged header declaring
    /// 512 MiB over-reserved 512 MiB before reading a single byte. This test
    /// pins the observed behavior so a future zip-crate upgrade that changes it
    /// (returning the actual size, which would neutralize the vector) fails
    /// loudly.
    #[test]
    fn entry_size_reports_declared_not_actual() {
        use std::io::Cursor;
        // Declare 64 MiB but supply only 8 real bytes.
        const DECLARED: u32 = 64 * 1024 * 1024;
        let real = b"realdata"; // 8 bytes
        let raw = forged_stored_zip("word/document.xml", real, DECLARED);
        let mut ar = zip::ZipArchive::new(Cursor::new(raw)).unwrap();
        let entry = ar.by_name("word/document.xml").unwrap();
        // The size field is read from the header at open time — BEFORE any
        // decompression / CRC check. It is the attacker's declared value.
        assert_eq!(
            entry.size(),
            DECLARED as u64,
            "entry.size() must report the declared (attacker-controlled) size — \
             if this fails, the zip crate now returns the actual size and RB11's \
             reserve-inflation vector no longer exists"
        );
    }

    #[test]
    fn initial_reserve_caps_the_declared_size() {
        // The reserve helper clamps an entry's declared size to INITIAL_RESERVE_CAP
        // so a forged 512 MiB declaration reserves at most the cap, not 512 MiB.
        // A small declared size is reserved exactly (no waste for legitimate files).
        assert_eq!(
            initial_reserve(8),
            8,
            "a small declared size is reserved exactly"
        );
        assert_eq!(
            initial_reserve(INITIAL_RESERVE_CAP as u64),
            INITIAL_RESERVE_CAP,
            "a declared size equal to the cap reserves the cap"
        );
        assert_eq!(
            initial_reserve(512 * 1024 * 1024),
            INITIAL_RESERVE_CAP,
            "a forged 512 MiB declaration is clamped to the cap, not honored"
        );
    }

    #[test]
    fn read_zip_bytes_reads_full_data_despite_huge_declared_reserve() {
        // A legitimately-authored entry whose real body is small but sits in an
        // archive is read in FULL regardless of the reserve cap — the cap only
        // bounds the up-front `with_capacity`; `read_to_end` grows as needed.
        // (Uses a normal entry: correctness of the read path is what we assert;
        // the anti-over-reserve property is covered by initial_reserve_caps_*.)
        let mut ar = archive_with("word/document.xml", b"<document>hello</document>");
        assert_eq!(
            read_zip_bytes(&mut ar, "word/document.xml").unwrap(),
            b"<document>hello</document>",
            "read must yield the complete real body, reserve cap notwithstanding"
        );
    }

    #[test]
    fn read_zip_helpers_reject_oversized_under_scoped_cap() {
        // 8-byte entry; a scoped cap of 4 must reject (never truncate) — the
        // zip-bomb guard applies to the open-archive helpers too.
        let mut ar = archive_with("ppt/media/big.bin", b"12345678");
        {
            let _guard = scoped_max(Some(4));
            let be = read_zip_bytes(&mut ar, "ppt/media/big.bin").unwrap_err();
            assert!(be.contains("exceeds size limit"), "got: {be}");
            let se = read_zip_string(&mut ar, "ppt/media/big.bin").unwrap_err();
            assert!(se.contains("exceeds size limit"), "got: {se}");
        }
        // Cap restored on guard drop → the same entry now reads in full.
        assert_eq!(
            read_zip_bytes(&mut ar, "ppt/media/big.bin").unwrap(),
            b"12345678"
        );
    }
}
