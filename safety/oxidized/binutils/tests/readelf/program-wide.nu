source ../helpers.nu

# Test readelf -lW: wide-format program headers
try { ^$env.REF -lW $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -lW $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -lW"
