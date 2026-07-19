source ../helpers.nu

# Test nm -P / --portability: POSIX format (name type value size)
try { ^$env.REF -P $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -P $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm --portability"
