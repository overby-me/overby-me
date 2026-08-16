source ../helpers.nu

# Test readelf -s (symbol table)
try { ^$env.REF -s $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -s $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -s (symbols)"
