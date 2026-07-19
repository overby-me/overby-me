source ../helpers.nu

# Test nm -A -P: POSIX format with file name prefix
try { ^$env.REF -A -P $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -A -P $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm -A -P"
