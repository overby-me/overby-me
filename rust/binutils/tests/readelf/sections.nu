source ../helpers.nu

# Test readelf -S (section headers)
try { ^$env.REF -S $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -S $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -S (section headers)"
