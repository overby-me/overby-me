source ../helpers.nu

# Test strings -t o: print offset in octal
try { ^$env.REF -t o $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -t o $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "strings -t o"
