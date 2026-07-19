source ../helpers.nu

# Test size -o: Berkeley format with octal radix for individual columns
try { ^$env.REF -o $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -o $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "size -o (octal radix)"
