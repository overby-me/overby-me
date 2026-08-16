source ../helpers.nu

# Test readelf -dA: dynamic + arch-specific (both empty for relocatables)
try { ^$env.REF -dA $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -dA $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -dA"
