source ../helpers.nu

# Test strings -t d: print offset in decimal
try { ^$env.REF -t d $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -t d $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "strings -t d"
