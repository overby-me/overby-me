source ../helpers.nu

# Test readelf -x .data: hex dump of .data section
try { ^$env.REF -x .data $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -x .data $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -x .data"
