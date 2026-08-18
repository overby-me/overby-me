source ../helpers.nu

# Test readelf -nW: Properties on the same line as the description (wide mode)
try { ^$env.REF -nW $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -nW $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -nW (inline properties)"
