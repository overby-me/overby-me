use std::env;

// The FFI wrapper includes perl_siphash.h and sbox32_hash.h from Perl's own
// source tree, to hash exactly as perl does. Those headers ship only in the
// source distribution, not in any installed perl, so the path to an unpacked
// tree has to come from outside.
//
// This used to be hardcoded to a directory under /tmp that a developer had
// unpacked by hand, which meant the crate did not build anywhere else,
// including in the monorepo's own Nix build. PERL_SRC now supplies it; the old
// path stays as the fallback so an existing checkout keeps working untouched.
const FALLBACK: &str = "/tmp/perl-tests-5.42/perl-5.42.0";

fn main() {
    let perl_src = env::var("PERL_SRC").unwrap_or_else(|_| FALLBACK.to_string());

    cc::Build::new()
        .file("src/perl_hash_wrapper.c")
        .include(&perl_src)
        .compile("perl_hash_wrapper");

    println!("cargo:rerun-if-changed=src/perl_hash_wrapper.c");
    println!("cargo:rerun-if-env-changed=PERL_SRC");
}
