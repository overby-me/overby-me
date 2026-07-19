source ../helpers.nu

# Test size -A (SysV format)
try { ^$env.REF -A $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -A $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "size -A (SysV format)"
