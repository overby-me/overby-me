//! Thin wrappers around `liblzma` providing buffered streaming
//! compress/decompress helpers.
//!
//! The decoder auto-detects the container format (xz, LZMA-alone,
//! and lzip) so callers don't need to know what they're handed
//! when in `Format::Auto`.

use std::io::{self, BufReader, BufWriter, Read, Write};

use liblzma::read::{XzDecoder, XzEncoder};
use liblzma::stream::{
    CONCATENATED, Filters, LzmaOptions, Stream, TELL_UNSUPPORTED_CHECK,
};

use crate::options::{BcjArch, FilterChain, FilterKind, Format};

/// Memory limit for decoders (bytes). `u64::MAX` means "no limit",
/// matching xz's `--memlimit-decompress=0` behaviour.
const DECODER_MEMLIMIT: u64 = u64::MAX;

/// .lzip magic: "LZIP" (`0x4C 0x5A 0x49 0x50`).
const LZIP_MAGIC: &[u8; 4] = b"LZIP";

/// Compress `input` to `output`. `level` is the preset 0-9 (used for
/// non-raw formats); `format` selects the container; `filter` is the
/// raw filter chain (required when `format == Format::Raw`).
pub fn compress_stream<R: Read, W: Write>(
    input: R,
    output: W,
    level: u32,
    format: Format,
    filter: Option<&FilterChain>,
) -> io::Result<()> {
    let mut writer = BufWriter::new(output);
    match format {
        Format::Xz | Format::Auto => {
            // Allow a custom filter chain even in xz-format mode.
            // Upstream `test_compress.sh` exercises the BCJ + LZMA2
            // chain via `--x86 --lzma2=…` against the xz container.
            if let Some(chain) = filter {
                let filters = build_filters(chain)?;
                let stream = Stream::new_stream_encoder(&filters, liblzma::stream::Check::Crc64)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
                let encoder = XzEncoder::new_stream(input, stream);
                let mut reader = BufReader::new(encoder);
                io::copy(&mut reader, &mut writer)?;
            } else {
                let encoder = XzEncoder::new(input, level);
                let mut reader = BufReader::new(encoder);
                io::copy(&mut reader, &mut writer)?;
            }
        }
        Format::Lzma => {
            let opts = LzmaOptions::new_preset(level)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            let stream = Stream::new_lzma_encoder(&opts)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            let encoder = XzEncoder::new_stream(input, stream);
            let mut reader = BufReader::new(encoder);
            io::copy(&mut reader, &mut writer)?;
        }
        Format::Lzip => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "lzip compression is not supported (decompression only)",
            ));
        }
        Format::Raw => {
            let chain = filter.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "raw format requires a filter chain (--lzma1=/--lzma2=/--filters=) and --suffix=",
                )
            })?;
            let filters = build_filters(chain)?;
            let stream = Stream::new_raw_encoder(&filters)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            let encoder = XzEncoder::new_stream(input, stream);
            let mut reader = BufReader::new(encoder);
            io::copy(&mut reader, &mut writer)?;
        }
    }
    writer.flush()?;
    Ok(())
}

#[allow(dead_code)]
pub fn decompress_stream<R: Read, W: Write>(input: R, output: W) -> io::Result<()> {
    decompress_stream_opts(input, output, false, Format::Auto, None)
}

/// Decompress with explicit options. `format` selects the decoder
/// (`Format::Auto` auto-detects; `Format::Raw` requires `filter`).
/// `no_warn` (matches xz's `-Q`) suppresses the unsupported-check
/// hard error.
pub fn decompress_stream_opts<R: Read, W: Write>(
    input: R,
    output: W,
    no_warn: bool,
    format: Format,
    filter: Option<&FilterChain>,
) -> io::Result<()> {
    let mut reader = BufReader::new(input);

    let stream = match format {
        Format::Raw => {
            let chain = filter.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "raw format requires a filter chain (--lzma1=/--lzma2=/--filters=) and --suffix=",
                )
            })?;
            let filters = build_filters(chain)?;
            Stream::new_raw_decoder(&filters)
        }
        Format::Lzip => Stream::new_lzip_decoder(DECODER_MEMLIMIT, decoder_flags(no_warn)),
        Format::Lzma => Stream::new_lzma_decoder(DECODER_MEMLIMIT),
        Format::Xz => Stream::new_stream_decoder(DECODER_MEMLIMIT, decoder_flags(no_warn)),
        Format::Auto => {
            // Peek the first 4 bytes to disambiguate lzip from xz/lzma.
            // `lzma_auto_decoder` only knows about .xz and .lzma magic.
            let buf = fill_peek_buf(&mut reader, 4)?;
            if buf.len() >= 4 && &buf[..4] == LZIP_MAGIC {
                Stream::new_lzip_decoder(DECODER_MEMLIMIT, decoder_flags(no_warn))
            } else {
                Stream::new_auto_decoder(DECODER_MEMLIMIT, decoder_flags(no_warn))
            }
        }
    }
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    let decoder = XzDecoder::new_stream(reader, stream);
    let mut decoded = BufReader::new(decoder);
    let mut writer = BufWriter::new(output);
    io::copy(&mut decoded, &mut writer)?;
    writer.flush()?;
    Ok(())
}

/// .xz magic header: `0xFD '7' 'z' 'X' 'Z' 0x00`
const XZ_MAGIC: &[u8; 6] = b"\xfd7zXZ\x00";

/// Returns true if `buf` looks like the start of a recognised
/// compressed stream (xz, lzip; lzma-alone has no reliable magic
/// so we don't try to detect it here).
pub fn looks_compressed(buf: &[u8]) -> bool {
    (buf.len() >= 6 && &buf[..6] == XZ_MAGIC)
        || (buf.len() >= 4 && &buf[..4] == LZIP_MAGIC)
}

/// Decompress in `-dfc` mode: like `decompress_stream_opts`, but if
/// the input does not look like a recognised compressed stream, copy
/// it verbatim to the output (matching xz's `-dfc` passthrough
/// behaviour).
pub fn decompress_or_passthrough<R: Read, W: Write>(
    input: R,
    output: W,
    no_warn: bool,
    format: Format,
    filter: Option<&FilterChain>,
) -> io::Result<()> {
    let mut reader = BufReader::new(input);
    let mut writer = BufWriter::new(output);

    if format == Format::Auto {
        let buf = fill_peek_buf(&mut reader, 6)?;
        if !looks_compressed(buf) {
            io::copy(&mut reader, &mut writer)?;
            writer.flush()?;
            return Ok(());
        }
    }

    let stream = match format {
        Format::Raw => {
            let chain = filter.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "raw format requires a filter chain (--lzma1=/--lzma2=/--filters=) and --suffix=",
                )
            })?;
            let filters = build_filters(chain)?;
            Stream::new_raw_decoder(&filters)
        }
        Format::Lzip => Stream::new_lzip_decoder(DECODER_MEMLIMIT, decoder_flags(no_warn)),
        Format::Lzma => Stream::new_lzma_decoder(DECODER_MEMLIMIT),
        Format::Xz | Format::Auto => {
            // Auto path: we already peeked and it looks compressed.
            let buf = reader.buffer();
            if buf.len() >= 4 && &buf[..4] == LZIP_MAGIC {
                Stream::new_lzip_decoder(DECODER_MEMLIMIT, decoder_flags(no_warn))
            } else {
                Stream::new_auto_decoder(DECODER_MEMLIMIT, decoder_flags(no_warn))
            }
        }
    }
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    let decoder = XzDecoder::new_stream(reader, stream);
    let mut decoded = BufReader::new(decoder);
    io::copy(&mut decoded, &mut writer)?;
    writer.flush()?;
    Ok(())
}

/// Build a liblzma `Filters` chain from our parsed `FilterChain`.
/// Validates that LZMA1/LZMA2 occur exactly once, as the final entry
/// (matching the on-the-wire constraint of every supported xz/raw
/// chain).
pub fn build_filters(chain: &FilterChain) -> io::Result<Filters> {
    if chain.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "empty filter chain",
        ));
    }
    let mut filters = Filters::new();
    let last = chain.as_slice().len() - 1;
    let mut lzma_seen = false;
    for (i, k) in chain.as_slice().iter().enumerate() {
        let is_last = i == last;
        match k {
            FilterKind::Lzma1Preset(p) | FilterKind::Lzma2Preset(p) => {
                if !is_last {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "LZMA1/LZMA2 must be the final filter in the chain",
                    ));
                }
                if lzma_seen {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "more than one LZMA1/LZMA2 filter in chain",
                    ));
                }
                lzma_seen = true;
                let opts = LzmaOptions::new_preset(*p)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
                match k {
                    FilterKind::Lzma1Preset(_) => {
                        filters.lzma1(&opts);
                    }
                    FilterKind::Lzma2Preset(_) => {
                        filters.lzma2(&opts);
                    }
                    _ => unreachable!(),
                }
            }
            FilterKind::Delta => {
                // Default distance is 1 (encoded as properties byte 0).
                filters
                    .delta_properties(&[0u8])
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            }
            FilterKind::Bcj(arch) => match arch {
                BcjArch::X86 => {
                    filters.x86();
                }
                BcjArch::Arm => {
                    filters.arm();
                }
                BcjArch::Arm64 => {
                    filters.arm64();
                }
                BcjArch::ArmThumb => {
                    filters.arm_thumb();
                }
                BcjArch::PowerPc => {
                    filters.powerpc();
                }
                BcjArch::Ia64 => {
                    filters.ia64();
                }
                BcjArch::Sparc => {
                    filters.sparc();
                }
                BcjArch::RiscV => {
                    filters.riscv();
                }
            },
        }
    }
    if !lzma_seen {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "filter chain must end with LZMA1 or LZMA2",
        ));
    }
    Ok(filters)
}

fn decoder_flags(no_warn: bool) -> u32 {
    if no_warn {
        CONCATENATED
    } else {
        CONCATENATED | TELL_UNSUPPORTED_CHECK
    }
}

/// Try to make at least `n` bytes available in the reader's buffer
/// without advancing the read position. Returns the buffered slice
/// (which may be shorter than `n` if the stream ends).
fn fill_peek_buf<R: Read>(reader: &mut BufReader<R>, n: usize) -> io::Result<&[u8]> {
    use std::io::BufRead;
    while reader.buffer().len() < n {
        let before = reader.buffer().len();
        reader.fill_buf()?;
        if reader.buffer().len() == before {
            break; // EOF or no progress
        }
    }
    Ok(reader.buffer())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::{BcjArch, FilterChain, FilterKind};

    fn chain_lzma1(preset: u32) -> FilterChain {
        let mut c = FilterChain::default();
        c.push(FilterKind::Lzma1Preset(preset));
        c
    }

    fn chain_lzma2(preset: u32) -> FilterChain {
        let mut c = FilterChain::default();
        c.push(FilterKind::Lzma2Preset(preset));
        c
    }

    fn chain_bcj_lzma2(arch: BcjArch, preset: u32) -> FilterChain {
        let mut c = FilterChain::default();
        c.push(FilterKind::Bcj(arch));
        c.push(FilterKind::Lzma2Preset(preset));
        c
    }

    fn roundtrip(payload: &[u8], level: u32, format: Format) {
        let mut compressed = Vec::new();
        compress_stream(payload, &mut compressed, level, format, None).expect("compress");
        let mut decompressed = Vec::new();
        decompress_stream(&compressed[..], &mut decompressed).expect("decompress");
        assert_eq!(decompressed, payload);
    }

    #[test]
    fn roundtrip_hello_world_xz() {
        roundtrip(b"hello, world!\n", 6, Format::Xz);
    }

    #[test]
    fn roundtrip_hello_world_lzma() {
        roundtrip(b"hello, world!\n", 6, Format::Lzma);
    }

    #[test]
    fn roundtrip_empty_xz() {
        let mut compressed = Vec::new();
        compress_stream(&b""[..], &mut compressed, 6, Format::Xz, None).unwrap();
        let mut decompressed = Vec::new();
        decompress_stream(&compressed[..], &mut decompressed).unwrap();
        assert_eq!(decompressed, b"");
    }

    #[test]
    fn roundtrip_every_preset_xz() {
        let payload: Vec<u8> = (0..4096u32).flat_map(|n| n.to_le_bytes()).collect();
        for level in 0..=9 {
            roundtrip(&payload, level, Format::Xz);
        }
    }

    #[test]
    fn roundtrip_every_preset_lzma() {
        let payload: Vec<u8> = (0..4096u32).flat_map(|n| n.to_le_bytes()).collect();
        for level in 0..=9 {
            roundtrip(&payload, level, Format::Lzma);
        }
    }

    #[test]
    fn decompress_rejects_garbage() {
        let mut sink = Vec::new();
        let err = decompress_stream(&b"not actually xz data"[..], &mut sink);
        assert!(err.is_err());
    }

    #[test]
    fn concatenated_xz_streams_decode() {
        let mut a = Vec::new();
        compress_stream(&b"AAA"[..], &mut a, 6, Format::Xz, None).unwrap();
        let mut b = Vec::new();
        compress_stream(&b"BBB"[..], &mut b, 6, Format::Xz, None).unwrap();
        a.extend_from_slice(&b);

        let mut decoded = Vec::new();
        decompress_stream(&a[..], &mut decoded).unwrap();
        assert_eq!(decoded, b"AAABBB");
    }

    #[test]
    fn lzip_magic_is_routed_to_lzip_decoder() {
        let mut sink = Vec::new();
        let err =
            decompress_stream(&b"LZIPnot-actually-a-valid-lzip-stream"[..], &mut sink).unwrap_err();
        let msg = err.to_string();
        assert!(
            !msg.contains("format not recognized"),
            "should have reached the lzip decoder, got: {msg}"
        );
    }

    #[test]
    fn roundtrip_raw_lzma1() {
        let payload = b"raw filter chain test payload that should compress nicely";
        let chain = chain_lzma1(0);
        let mut compressed = Vec::new();
        compress_stream(&payload[..], &mut compressed, 0, Format::Raw, Some(&chain)).unwrap();
        let mut decompressed = Vec::new();
        decompress_stream_opts(
            &compressed[..],
            &mut decompressed,
            false,
            Format::Raw,
            Some(&chain),
        )
        .unwrap();
        assert_eq!(&decompressed[..], &payload[..]);
    }

    #[test]
    fn roundtrip_raw_lzma2() {
        let payload = b"raw lzma2 filter chain test payload";
        let chain = chain_lzma2(0);
        let mut compressed = Vec::new();
        compress_stream(&payload[..], &mut compressed, 0, Format::Raw, Some(&chain)).unwrap();
        let mut decompressed = Vec::new();
        decompress_stream_opts(
            &compressed[..],
            &mut decompressed,
            false,
            Format::Raw,
            Some(&chain),
        )
        .unwrap();
        assert_eq!(&decompressed[..], &payload[..]);
    }

    #[test]
    fn roundtrip_xz_with_bcj_x86_chain() {
        let payload: Vec<u8> = (0..8192u32).flat_map(|n| n.to_le_bytes()).collect();
        let chain = chain_bcj_lzma2(BcjArch::X86, 4);
        let mut compressed = Vec::new();
        compress_stream(&payload[..], &mut compressed, 4, Format::Xz, Some(&chain)).unwrap();
        let mut decompressed = Vec::new();
        decompress_stream(&compressed[..], &mut decompressed).unwrap();
        assert_eq!(decompressed, payload);
    }

    #[test]
    fn roundtrip_xz_with_bcj_arm64_chain() {
        let payload: Vec<u8> = (0..8192u32).flat_map(|n| n.to_le_bytes()).collect();
        let chain = chain_bcj_lzma2(BcjArch::Arm64, 4);
        let mut compressed = Vec::new();
        compress_stream(&payload[..], &mut compressed, 4, Format::Xz, Some(&chain)).unwrap();
        let mut decompressed = Vec::new();
        decompress_stream(&compressed[..], &mut decompressed).unwrap();
        assert_eq!(decompressed, payload);
    }

    #[test]
    fn roundtrip_xz_with_bcj_riscv_chain() {
        let payload: Vec<u8> = (0..8192u32).flat_map(|n| n.to_le_bytes()).collect();
        let chain = chain_bcj_lzma2(BcjArch::RiscV, 4);
        let mut compressed = Vec::new();
        compress_stream(&payload[..], &mut compressed, 4, Format::Xz, Some(&chain)).unwrap();
        let mut decompressed = Vec::new();
        decompress_stream(&compressed[..], &mut decompressed).unwrap();
        assert_eq!(decompressed, payload);
    }

    #[test]
    fn roundtrip_xz_with_delta_chain() {
        // Delta + LZMA2 — exercises the non-BCJ helper path.
        let payload: Vec<u8> = (0..4096u32).flat_map(|n| (n as u8).to_le_bytes()).collect();
        let mut chain = FilterChain::default();
        chain.push(FilterKind::Delta);
        chain.push(FilterKind::Lzma2Preset(4));
        let mut compressed = Vec::new();
        compress_stream(&payload[..], &mut compressed, 4, Format::Xz, Some(&chain)).unwrap();
        let mut decompressed = Vec::new();
        decompress_stream(&compressed[..], &mut decompressed).unwrap();
        assert_eq!(decompressed, payload);
    }

    #[test]
    fn build_filters_rejects_lzma_not_last() {
        let mut chain = FilterChain::default();
        chain.push(FilterKind::Lzma2Preset(0));
        chain.push(FilterKind::Bcj(BcjArch::X86));
        match build_filters(&chain) {
            Ok(_) => panic!("expected build_filters to fail"),
            Err(e) => assert_eq!(e.kind(), io::ErrorKind::InvalidInput),
        }
    }

    #[test]
    fn build_filters_rejects_chain_without_lzma() {
        let mut chain = FilterChain::default();
        chain.push(FilterKind::Bcj(BcjArch::X86));
        match build_filters(&chain) {
            Ok(_) => panic!("expected build_filters to fail"),
            Err(e) => assert_eq!(e.kind(), io::ErrorKind::InvalidInput),
        }
    }

    #[test]
    fn build_filters_rejects_two_lzma() {
        let mut chain = FilterChain::default();
        chain.push(FilterKind::Lzma1Preset(0));
        chain.push(FilterKind::Lzma2Preset(0));
        match build_filters(&chain) {
            Ok(_) => panic!("expected build_filters to fail"),
            Err(e) => assert_eq!(e.kind(), io::ErrorKind::InvalidInput),
        }
    }

    #[test]
    fn raw_compress_without_filter_errors() {
        let mut sink = Vec::new();
        let err = compress_stream(&b"x"[..], &mut sink, 6, Format::Raw, None).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn raw_decompress_without_filter_errors() {
        let mut sink = Vec::new();
        let err =
            decompress_stream_opts(&b"x"[..], &mut sink, false, Format::Raw, None).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
