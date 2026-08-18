source ../helpers.nu

# Test size -x: Berkeley format with hex radix for individual columns
try { ^$env.REF -x $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -x $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "size -x (hex radix)"
