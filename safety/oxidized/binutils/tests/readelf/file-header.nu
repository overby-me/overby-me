source ../helpers.nu

# Test readelf -h (file header)
try { ^$env.REF -h $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -h $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -h (file header)"
