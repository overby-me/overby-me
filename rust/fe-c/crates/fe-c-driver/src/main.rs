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

extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_index;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_public;
extern crate rustc_span;
extern crate thin_vec;

mod census;
mod instrument;
mod provenance;

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

    // Instrumentation mode (B2): rewrite MIR to inject cementite checks.
    // Selected by FEC_INSTRUMENT so ordinary census/probe invocations are
    // unaffected; it uses a rustc_driver::Callbacks driver rather than the
    // read-only rustc_public one.
    if std::env::var_os("FEC_INSTRUMENT").is_some() {
        inject_cementite(&mut args);
        std::process::exit(instrument::run(&args));
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

/// Makes `cementite` available as an extern crate to *every* compilation so
/// the injected check calls resolve — including in third-party dependencies
/// (e.g. `smallvec`) that do not declare it. `FEC_CEMENTITE_RLIB` points at
/// a prebuilt `libcementite.rlib` and `FEC_CEMENTITE_DEPS` at its
/// dependency search dir. This is the minimal `cargo-fe-c` orchestration:
/// cementite behaves like an always-available sysroot crate.
fn inject_cementite(args: &mut Vec<String>) {
    let Ok(rlib) = std::env::var("FEC_CEMENTITE_RLIB") else {
        return;
    };
    // Never inject into cementite's own build, and never duplicate an
    // existing `--extern cementite`.
    if crate_name(args).as_deref() == Some("cementite")
        || args.iter().any(|a| a.contains("cementite="))
    {
        return;
    }
    // `force:` loads cementite even when the crate's source never references
    // it (dependencies like smallvec don't), so the check fn is resolvable
    // and the injected calls link. The modifier needs -Zunstable-options.
    if !args.iter().any(|a| a == "-Zunstable-options") {
        args.push("-Zunstable-options".to_string());
    }
    args.push("--extern".to_string());
    args.push(format!("force:cementite={rlib}"));
    if let Ok(deps) = std::env::var("FEC_CEMENTITE_DEPS") {
        args.push("-L".to_string());
        args.push(format!("dependency={deps}"));
    }
}

/// Extracts the `--crate-name` value from a rustc command line.
fn crate_name(args: &[String]) -> Option<String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--crate-name" {
            return it.next().cloned();
        }
        if let Some(v) = a.strip_prefix("--crate-name=") {
            return Some(v.to_string());
        }
    }
    None
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
