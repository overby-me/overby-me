source ../helpers.nu

# Test readelf -hS: file header + section headers
try { ^$env.REF -hS $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -hS $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -hS"
