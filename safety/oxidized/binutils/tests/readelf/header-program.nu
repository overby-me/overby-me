source ../helpers.nu

# Test readelf -hl: file header + program headers
try { ^$env.REF -hl $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -hl $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -hl"
