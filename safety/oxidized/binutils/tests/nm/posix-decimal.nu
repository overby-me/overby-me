source ../helpers.nu

# Test nm -P -t d: POSIX format with decimal radix
try { ^$env.REF -P -t d $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -P -t d $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm -P -t d"
