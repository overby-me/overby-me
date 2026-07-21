//! Compiles the C test harness (`tests/harness.c`) when the `interpose`
//! feature is on, so `tests/interpose_c.rs` can prove libc interposition
//! applies to a genuinely C-compiled translation unit. Does nothing
//! otherwise, so ordinary builds pull in no C toolchain work.

fn main() {
    println!("cargo::rerun-if-changed=tests/harness.c");
    println!("cargo::rerun-if-changed=build.rs");

    if std::env::var_os("CARGO_FEATURE_INTERPOSE").is_some() {
        cc::Build::new()
            .file("tests/harness.c")
            // The harness is a trivial malloc shim; fortify (which glibc
            // headers warn about at -O0) buys nothing and only adds noise.
            .define("_FORTIFY_SOURCE", "0")
            .compile("fec_harness");
    }
}
