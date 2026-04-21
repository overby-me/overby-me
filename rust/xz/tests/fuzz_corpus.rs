//! Decoder-stability test: feed every file in the upstream xz
//! `tests/files/` corpus (and every truncated and 1-byte-flipped
//! variant) into our decoder and assert it never panics.
//!
//! This is the in-tree analogue of OSS-Fuzz's `fuzz_decode_stream`
//! and `fuzz_decode_alone` targets — it doesn't replace a real
//! libFuzzer run, but it catches the easy class of bugs (panics on
//! attacker-controlled input) without needing a nightly toolchain
//! or a libFuzzer harness.
//!
//! The corpus directory is located via the `RUST_XZ_FUZZ_CORPUS`
//! environment variable. The `rust-xz-fuzz` Nix check sets this to
//! the unpacked `xz-*/tests/files/` directory; if the variable is
//! unset (the local-`cargo test` case) the test is skipped.

use std::fs;
use std::io::sink;
use std::path::PathBuf;

use rust_xz::codec::decompress_stream;

fn corpus_dir() -> Option<PathBuf> {
    std::env::var_os("RUST_XZ_FUZZ_CORPUS").map(PathBuf::from)
}

fn collect_inputs(dir: &PathBuf) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => panic!("could not read fuzz corpus dir {dir:?}: {e}"),
    };
    for ent in entries.flatten() {
        let path = ent.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Only fuzz on the file types the decoder claims to handle.
        if !(name.ends_with(".xz") || name.ends_with(".lzma") || name.ends_with(".lz")) {
            continue;
        }
        match fs::read(&path) {
            Ok(bytes) => out.push((name, bytes)),
            Err(e) => panic!("read {path:?}: {e}"),
        }
    }
    out
}

fn decode_no_panic(bytes: &[u8]) {
    // We don't care about success/failure; the only thing that
    // matters is that the call returns instead of aborting. Wrap
    // in `catch_unwind` so a panic still fails the test cleanly
    // (rather than terminating the whole test runner).
    let result = std::panic::catch_unwind(|| {
        let _ = decompress_stream(bytes, &mut sink());
    });
    assert!(result.is_ok(), "decoder panicked on input ({} bytes)", bytes.len());
}

#[test]
fn corpus_decodes_without_panicking() {
    let Some(dir) = corpus_dir() else {
        eprintln!("skipping: RUST_XZ_FUZZ_CORPUS not set");
        return;
    };
    let inputs = collect_inputs(&dir);
    assert!(!inputs.is_empty(), "fuzz corpus dir {dir:?} contained no inputs");

    for (name, bytes) in &inputs {
        // 1) Original input.
        decode_no_panic(bytes);

        // 2) Truncated at every 64-byte boundary, plus the empty prefix.
        let step = 64usize;
        let mut len = 0usize;
        while len < bytes.len() {
            decode_no_panic(&bytes[..len]);
            len = len.saturating_add(step);
        }

        // 3) One-byte-flipped variants at the same boundaries.
        let mut idx = 0usize;
        while idx < bytes.len() {
            let mut mutated = bytes.clone();
            mutated[idx] ^= 0xff;
            decode_no_panic(&mutated);
            idx = idx.saturating_add(step.max(1));
        }
        eprintln!("fuzzed {name} ({} bytes, OK)", bytes.len());
    }
}
