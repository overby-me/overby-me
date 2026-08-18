source ../helpers.nu

# Test readelf -gW: wide format section groups
try { ^$env.REF -gW $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -gW $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -gW"
