source ../helpers.nu

# Test readelf -e: file header + section headers + program headers
try { ^$env.REF -e $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -e $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -e (all headers)"
