source ../helpers.nu

# Test nm -uP: undefined-only with POSIX format
try { ^$env.REF -uP $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -uP $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm -uP"
