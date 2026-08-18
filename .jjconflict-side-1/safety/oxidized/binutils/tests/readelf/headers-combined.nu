source ../helpers.nu

# Test readelf -hSl: file header + sections + program headers (long form)
try { ^$env.REF -hSl $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -hSl $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -hSl"
