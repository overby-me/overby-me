source ../helpers.nu

# Test readelf -u: unwind information dump (no processor-specific decoder)
try { ^$env.REF -u $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -u $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -u"
