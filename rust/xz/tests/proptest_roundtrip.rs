//! Property-based round-trip tests for the codec.
//!
//! Each generator hands a random byte buffer to the chosen
//! container/preset combination and asserts that the decoded output
//! exactly equals the input. The shrinker is left at the proptest
//! defaults — failures will minimise to the smallest reproducer.

use proptest::prelude::*;
use rust_xz::codec::{compress_stream, decompress_stream, decompress_stream_opts};
use rust_xz::options::{BcjArch, FilterChain, FilterKind, Format};

/// Buffers up to ~32 KiB so each iteration finishes in well under a
/// second even at preset 9.
fn payload_strategy() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..32 * 1024)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    #[test]
    fn xz_roundtrip_arbitrary_buffer(payload in payload_strategy(), level in 0u32..=4) {
        let mut compressed = Vec::new();
        compress_stream(&payload[..], &mut compressed, level, Format::Xz, None).unwrap();
        let mut decoded = Vec::new();
        decompress_stream(&compressed[..], &mut decoded).unwrap();
        prop_assert_eq!(decoded, payload);
    }

    #[test]
    fn lzma_roundtrip_arbitrary_buffer(payload in payload_strategy(), level in 0u32..=4) {
        let mut compressed = Vec::new();
        compress_stream(&payload[..], &mut compressed, level, Format::Lzma, None).unwrap();
        let mut decoded = Vec::new();
        decompress_stream(&compressed[..], &mut decoded).unwrap();
        prop_assert_eq!(decoded, payload);
    }

    #[test]
    fn raw_lzma2_roundtrip_arbitrary_buffer(payload in payload_strategy()) {
        let mut chain = FilterChain::default();
        chain.push(FilterKind::Lzma2Preset(0));
        let mut compressed = Vec::new();
        compress_stream(&payload[..], &mut compressed, 0, Format::Raw, Some(&chain)).unwrap();
        let mut decoded = Vec::new();
        decompress_stream_opts(
            &compressed[..],
            &mut decoded,
            false,
            Format::Raw,
            Some(&chain),
        )
        .unwrap();
        prop_assert_eq!(decoded, payload);
    }

    #[test]
    fn bcj_x86_lzma2_roundtrip_arbitrary_buffer(payload in payload_strategy()) {
        let mut chain = FilterChain::default();
        chain.push(FilterKind::Bcj(BcjArch::X86));
        chain.push(FilterKind::Lzma2Preset(0));
        let mut compressed = Vec::new();
        compress_stream(&payload[..], &mut compressed, 0, Format::Xz, Some(&chain)).unwrap();
        let mut decoded = Vec::new();
        decompress_stream(&compressed[..], &mut decoded).unwrap();
        prop_assert_eq!(decoded, payload);
    }

    #[test]
    fn decompressing_garbage_never_panics(garbage in prop::collection::vec(any::<u8>(), 0..1024)) {
        // The decoder must not panic on arbitrary input — it may
        // succeed (vanishingly unlikely) or return an error, but
        // it must never abort the process.
        let mut sink = Vec::new();
        let _ = decompress_stream(&garbage[..], &mut sink);
    }
}
