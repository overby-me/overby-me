fn main() {
    cc::Build::new()
        .file("src/perl_hash_wrapper.c")
        .include("/tmp/perl-tests-5.42/perl-5.42.0")
        .compile("perl_hash_wrapper");
    println!("cargo:rerun-if-changed=src/perl_hash_wrapper.c");
}
