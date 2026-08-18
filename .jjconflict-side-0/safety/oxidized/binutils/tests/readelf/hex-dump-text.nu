source ../helpers.nu

# Test readelf -x .text: should print the "section has relocations" NOTE
try { ^$env.REF -x .text $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -x .text $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -x .text (relocations note)"
