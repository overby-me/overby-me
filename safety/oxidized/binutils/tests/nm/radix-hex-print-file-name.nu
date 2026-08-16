source ../helpers.nu

# Test nm -t x -A: combined hex radix and file-name prefix
try { ^$env.REF -t x -A $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -t x -A $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm -t x -A"
