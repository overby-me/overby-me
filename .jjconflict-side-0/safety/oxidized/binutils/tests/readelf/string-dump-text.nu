source ../helpers.nu

# Test readelf -p .text: should print the "section has relocations" NOTE
try { ^$env.REF -p .text $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -p .text $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -p .text (relocations note)"
