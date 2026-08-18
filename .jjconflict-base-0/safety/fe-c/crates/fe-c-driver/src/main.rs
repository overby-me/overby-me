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

extern crate rustc_ast;
extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_index;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_parse;
extern crate rustc_public;
extern crate rustc_session;
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
        let name = crate_name(&args);
        // Never instrument:
        // - cementite itself: it *defines* the __fec_* symbols, so
        //   instrumenting it would recurse and clash with the injected decls;
        // - build scripts and proc-macro crates: they run at *build* time on
        //   the host and are never linked against cementite, so injected
        //   __fec_* calls would be undefined symbols. This matters under
        //   whole-graph instrumentation, where cementite's own build script
        //   is in the graph.
        // FEC_INSTRUMENT_ONLY further scopes instrumentation to a crate-name
        // list (the crate under test); whole-graph (the default) works
        // because injection is symbol-level (A4b) with no dependency edge.
        let instrument_this = name.as_deref() != Some("cementite")
            && !is_host_tool(&args, name.as_deref())
            && match std::env::var("FEC_INSTRUMENT_ONLY") {
                Ok(list) => list.split(',').any(|n| Some(n) == name.as_deref()),
                Err(_) => true,
            };
        if instrument_this {
            inject_cementite(&mut args);
            std::process::exit(instrument::run(&args));
        }
        // Not in scope: compile normally, no instrumentation, no cementite.
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

/// Links `cementite` once into the final binary so the injected `extern "C"`
/// check symbols resolve (A4b, the ASan model). No `--extern`, no crate
/// dependency edge: each instrumented crate only *declares* the symbols
/// (see `inject_fec_decls`); only the final binary/test needs cementite's
/// object. `FEC_CEMENTITE_RLIB` points at a prebuilt `libcementite.rlib`,
/// which the linker treats as an archive and pulls the `__fec_*` members
/// from. cementite is dependency-free (I11), so no extra `-L` is needed.
fn inject_cementite(args: &mut Vec<String>) {
    let Ok(rlib) = std::env::var("FEC_CEMENTITE_RLIB") else {
        return;
    };
    if crate_name(args).as_deref() == Some("cementite") {
        return;
    }
    // If cementite is already a Cargo dependency (`--extern cementite=…`, as
    // in the harness that uses `FecAlloc`), cargo links it — a second copy
    // via link-arg would duplicate every symbol.
    if args.iter().any(|a| a.starts_with("cementite=")) {
        return;
    }
    if produces_binary(args) {
        args.push("-C".to_string());
        args.push(format!("link-arg={rlib}"));
    }
}

/// Whether this rustc invocation produces a linked binary (a `bin` crate or
/// a `--test` harness), as opposed to an intermediate rlib.
fn produces_binary(args: &[String]) -> bool {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--test" {
            return true;
        }
        let ct = if a == "--crate-type" {
            it.next().map(String::as_str)
        } else {
            a.strip_prefix("--crate-type=")
        };
        if let Some(ct) = ct
            && ct.split(',').any(|t| t == "bin")
        {
            return true;
        }
    }
    false
}

/// Whether this invocation compiles host-time code that is never linked
/// against cementite: a Cargo build script (`--crate-name build_script_*`) or
/// a proc-macro crate (`--crate-type proc-macro`). Instrumenting either would
/// emit `__fec_*` calls with nothing to resolve them.
fn is_host_tool(args: &[String], name: Option<&str>) -> bool {
    if name.is_some_and(|n| n.starts_with("build_script_")) {
        return true;
    }
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let ct = if a == "--crate-type" {
            it.next().map(String::as_str)
        } else {
            a.strip_prefix("--crate-type=")
        };
        if let Some(ct) = ct
            && ct.split(',').any(|t| t == "proc-macro")
        {
            return true;
        }
    }
    false
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
