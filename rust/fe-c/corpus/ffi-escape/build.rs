//! Compiles the C harness (`ffi_harness.c`) into a static library the fixture
//! links, so the RUSTSEC-2021-0128 shape runs against genuinely C-compiled
//! code. Invokes `cc`/`ar` directly (no `cc` crate) so the fixture drags in
//! no build-toolchain crate the whole-graph instrumenter would try to link.

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=ffi_harness.c");
    println!("cargo::rerun-if-changed=build.rs");

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR is set for build scripts");
    let obj = Path::new(&out_dir).join("ffi_harness.o");
    let archive = Path::new(&out_dir).join("libffi_harness.a");

    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let compile = Command::new(&cc)
        .args(["-c", "-fPIC"])
        .arg("ffi_harness.c")
        .arg("-o")
        .arg(&obj)
        .status()
        .expect("spawn C compiler");
    assert!(compile.success(), "compiling ffi_harness.c with {cc} failed");

    let ar = std::env::var("AR").unwrap_or_else(|_| "ar".to_string());
    let archive_status = Command::new(&ar)
        .arg("rcs")
        .arg(&archive)
        .arg(&obj)
        .status()
        .expect("spawn ar");
    assert!(archive_status.success(), "archiving libffi_harness.a with {ar} failed");

    println!("cargo::rustc-link-search=native={out_dir}");
    println!("cargo::rustc-link-lib=static=ffi_harness");
}
