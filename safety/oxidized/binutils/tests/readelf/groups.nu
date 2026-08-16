source ../helpers.nu

# Test readelf -g / --section-groups: display section groups
try { ^$env.REF -g $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -g $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -g (section groups)"
