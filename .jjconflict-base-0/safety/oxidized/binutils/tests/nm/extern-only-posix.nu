source ../helpers.nu

# Test nm -gP: extern-only with POSIX format
try { ^$env.REF -gP $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -gP $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm -gP"
