//! Compiles the C test harness (`tests/harness.c`) when the `interpose`
//! feature is on, so `tests/interpose_c.rs` can prove libc interposition
//! applies to a genuinely C-compiled translation unit. Does nothing
//! otherwise, so ordinary builds pull in no C toolchain work.
//!
//! It invokes `cc` and `ar` directly rather than via the `cc` crate:
//! cementite is freestanding (invariant I11) and carries no dependencies at
//! all, not even build-time ones, so a crate that merely installs `FecAlloc`
//! never drags a C-toolchain crate (and its build script) into its graph —
//! which the whole-graph instrumenter would otherwise try, and fail, to
//! link against cementite.

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=tests/harness.c");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=CC");
    println!("cargo::rerun-if-env-changed=AR");

    if std::env::var_os("CARGO_FEATURE_INTERPOSE").is_none() {
        return;
    }

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR is set for build scripts");
    let obj = Path::new(&out_dir).join("fec_harness.o");
    let archive = Path::new(&out_dir).join("libfec_harness.a");

    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let compile = Command::new(&cc)
        .args(["-c", "-fPIC"])
        // The harness is a trivial malloc shim; fortify (which glibc headers
        // warn about at -O0) buys nothing and only adds noise.
        .args(["-D_FORTIFY_SOURCE=0"])
        .arg("tests/harness.c")
        .arg("-o")
        .arg(&obj)
        .status()
        .expect("spawn C compiler");
    assert!(
        compile.success(),
        "compiling tests/harness.c with {cc} failed"
    );

    let ar = std::env::var("AR").unwrap_or_else(|_| "ar".to_string());
    let archive_status = Command::new(&ar)
        .arg("rcs")
        .arg(&archive)
        .arg(&obj)
        .status()
        .expect("spawn ar");
    assert!(
        archive_status.success(),
        "archiving libfec_harness.a with {ar} failed"
    );

    println!("cargo::rustc-link-search=native={out_dir}");
    println!("cargo::rustc-link-lib=static=fec_harness");
}
