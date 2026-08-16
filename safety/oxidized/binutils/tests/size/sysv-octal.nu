source ../helpers.nu

# Test size -A -o: sysv format with octal radix
try { ^$env.REF -A -o $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -A -o $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "size -A -o"
