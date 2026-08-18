source ../helpers.nu

# Test readelf -nh: combined notes + file header
try { ^$env.REF -nh $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -nh $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -nh"
