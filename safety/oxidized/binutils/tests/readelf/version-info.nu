source ../helpers.nu

# Test readelf -V / --version-info: display version sections (or
# "No version information" message)
try { ^$env.REF -V $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -V $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -V"
