source ../helpers.nu

# Test nm -t x: print symbol values in hex (default)
try { ^$env.REF -t x $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -t x $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm -t x (hex radix)"
