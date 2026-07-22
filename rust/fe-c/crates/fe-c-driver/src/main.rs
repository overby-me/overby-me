//! `fe-c-driver`: rustc-as-a-library on `rustc_public` (the stable-MIR
//! surface). Task A5 gives it a **visitation census**: it drives a normal
//! compilation and, once MIR exists, reports every pointer-typed local,
//! every dereference, every raw->safe cast, and every FFI edge — the
//! I1 "total visitation" inventory that later tasks turn into checks.
//!
//! No rewriting yet (that is Task B2). The driver behaves like `rustc`: it
//! is invoked with a rustc command line and compiles the crate, emitting
//! the census as JSON to the path in `FEC_CENSUS_OUT` (or stderr).

#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_public;

mod census;

use std::ops::ControlFlow;

fn main() {
    let mut args: Vec<String> = std::env::args().collect();

    // Default real compiler when used as `RUSTC` directly.
    let mut real_rustc = "rustc".to_string();

    // Support the `RUSTC_WRAPPER` convention: cargo invokes the wrapper as
    // `fe-c-driver <path-to-rustc> <rustc args…>`. Remember and drop the
    // interposed rustc path so what remains is a plain rustc command line,
    // exactly as when used as `RUSTC` directly.
    if args.len() > 1 {
        let a1 = std::path::Path::new(&args[1]);
        let looks_like_rustc = a1
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s == "rustc" || s == "fe-c-driver");
        if looks_like_rustc {
            real_rustc = args.remove(1);
        }
    }

    // cargo probes the compiler with info-only flags (`-vV`, `--version`,
    // `--print …`) before every build. rustc_public's driver cannot serve
    // those, and the output must be byte-identical to real rustc's for
    // cargo to parse the host triple and cfgs — so delegate them verbatim.
    if is_info_probe(&args[1..]) {
        let status = std::process::Command::new(&real_rustc)
            .args(&args[1..])
            .status();
        std::process::exit(status.ok().and_then(|s| s.code()).unwrap_or(1));
    }

    let run = rustc_public::run!(&args, census_callback);

    if run.is_err() {
        std::process::exit(1);
    }
}

/// Whether these rustc args are an information probe (no compilation), which
/// must be answered by the real compiler.
fn is_info_probe(args: &[String]) -> bool {
    args.iter()
        .any(|a| matches!(a.as_str(), "-vV" | "-V" | "--version") || a.starts_with("--print"))
}

/// Runs the census after the compiler has produced MIR. Returns
/// `ControlFlow::Continue` so compilation still finishes normally (A5 does
/// not rewrite).
fn census_callback() -> ControlFlow<()> {
    // Belt and braces: even if the census itself panics, never break the
    // compilation — Continue unconditionally. A5 is read-only.
    let result =
        std::panic::catch_unwind(|| census::run().map_err(|e| eprintln!("fe-c-driver: {e}")));
    if result.is_err() {
        eprintln!("fe-c-driver: census panicked; compilation continues uninstrumented");
    }
    ControlFlow::Continue(())
}
