source ../helpers.nu

# Test readelf -ns: notes + symbols
try { ^$env.REF -ns $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -ns $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -ns"
