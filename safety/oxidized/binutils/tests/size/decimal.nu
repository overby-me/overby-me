source ../helpers.nu

# Test size -d: Berkeley format with decimal radix (default already)
try { ^$env.REF -d $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -d $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "size -d (decimal radix)"
