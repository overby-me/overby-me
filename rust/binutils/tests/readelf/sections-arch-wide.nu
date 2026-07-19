source ../helpers.nu

# Test readelf -SAW: sections + arch-specific + wide
try { ^$env.REF -SAW $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -SAW $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -SAW"
