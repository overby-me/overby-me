source ../helpers.nu

# Test nm -j -A: `-j` should drop the file-name prefix from `-A`
try { ^$env.REF -j -A $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -j -A $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm -j -A"
