source ../helpers.nu

# Test readelf -S -W: wide format section headers
try { ^$env.REF -S -W $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -S -W $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -S -W"
