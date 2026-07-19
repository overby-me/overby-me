source ../helpers.nu

# Test nm -t d: print symbol values in decimal
try { ^$env.REF -t d $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -t d $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm -t d (decimal radix)"
