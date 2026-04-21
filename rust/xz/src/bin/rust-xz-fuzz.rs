//! Decoder-stability fuzz harness packaged as a standalone binary so
//! the Nix `rust-xz-fuzz` check can run it without a vendored cargo
//! registry inside the build sandbox.
//!
//! Behaviour:
//! * Reads the corpus directory from `argv[1]`, falling back to
//!   `$RUST_XZ_FUZZ_CORPUS`.
//! * For every `*.xz` / `*.lzma` / `*.lz` file in the directory, feeds
//!   the file's bytes (and prefix-truncated and 1-byte-flipped
//!   variants) into the rust-xz decoder.
//! * Asserts that the decoder never panics. Decode failures are fine
//!   (most mutations should fail) — the only failure mode that
//!   matters for this harness is an actual abort.
//!
//! Exits 0 on success, 1 on any panic, 2 on usage / corpus errors.
//! Mirrors the in-tree `tests/fuzz_corpus.rs` integration test.

use std::fs;
use std::io::sink;
use std::path::PathBuf;
use std::process::ExitCode;

use rust_xz::codec::decompress_stream;

fn collect_inputs(dir: &PathBuf) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for ent in fs::read_dir(dir).expect("read corpus dir").flatten() {
        let path = ent.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !(name.ends_with(".xz") || name.ends_with(".lzma") || name.ends_with(".lz")) {
            continue;
        }
        if let Ok(bytes) = fs::read(&path) {
            out.push((name, bytes));
        }
    }
    out
}

fn decode_no_panic(bytes: &[u8]) -> bool {
    std::panic::catch_unwind(|| {
        let _ = decompress_stream(bytes, &mut sink());
    })
    .is_ok()
}

fn main() -> ExitCode {
    let dir: PathBuf = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("RUST_XZ_FUZZ_CORPUS").ok())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            eprintln!("usage: rust-xz-fuzz <corpus-dir>");
            std::process::exit(2);
        });

    let inputs = collect_inputs(&dir);
    if inputs.is_empty() {
        eprintln!("no corpus inputs found in {dir:?}");
        return ExitCode::from(2);
    }
    eprintln!("loaded {} corpus inputs from {dir:?}", inputs.len());

    let mut any_panic = false;
    for (name, bytes) in &inputs {
        if !decode_no_panic(bytes) {
            eprintln!("PANIC on {name} (full input)");
            any_panic = true;
        }
        let step = 64usize;
        let mut len = 0usize;
        while len < bytes.len() {
            if !decode_no_panic(&bytes[..len]) {
                eprintln!("PANIC on {name} (truncated to {len} bytes)");
                any_panic = true;
            }
            len = len.saturating_add(step);
        }
        let mut idx = 0usize;
        while idx < bytes.len() {
            let mut mutated = bytes.clone();
            mutated[idx] ^= 0xff;
            if !decode_no_panic(&mutated) {
                eprintln!("PANIC on {name} (byte {idx} flipped)");
                any_panic = true;
            }
            idx = idx.saturating_add(step.max(1));
        }
        eprintln!("ok: {name} ({} bytes)", bytes.len());
    }

    if any_panic {
        eprintln!("FAIL: at least one input panicked the decoder");
        ExitCode::from(1)
    } else {
        eprintln!("OK: {} inputs survived all mutations without panicking", inputs.len());
        ExitCode::SUCCESS
    }
}
