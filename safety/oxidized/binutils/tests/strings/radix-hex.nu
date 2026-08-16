source ../helpers.nu

# Test strings -t x: print offset in hex
try { ^$env.REF -t x $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -t x $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "strings -t x"
