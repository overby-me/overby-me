source ../helpers.nu

# Test readelf -dl: both program headers and dynamic section messages emitted
try { ^$env.REF -dl $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -dl $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -dl (no return after no-program-headers)"
