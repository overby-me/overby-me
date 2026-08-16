source ../helpers.nu

# Test readelf -r / --relocs: display relocations
try { ^$env.REF -r $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -r $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -r"
