source ../helpers.nu

# Test nm -t o: print symbol values in octal
try { ^$env.REF -t o $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -t o $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm -t o (octal radix)"
