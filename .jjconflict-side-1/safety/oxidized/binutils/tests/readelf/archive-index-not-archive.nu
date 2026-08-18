source ../helpers.nu

# Test readelf -c on a non-archive: should print the "not an archive" error
try { ^$env.REF -c $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -c $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -c on non-archive"
