source ../helpers.nu

# Test nm -P -t o: POSIX format with octal radix
try { ^$env.REF -P -t o $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -P -t o $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm -P -t o"
