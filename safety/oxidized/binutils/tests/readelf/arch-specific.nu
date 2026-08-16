source ../helpers.nu

# Test readelf -A / --arch-specific: display architecture-specific info (none for x86)
try { ^$env.REF -A $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -A $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -A"
