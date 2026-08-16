source ../helpers.nu

# Test size -A -x: sysv format with hex radix
try { ^$env.REF -A -x $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -A -x $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "size -A -x"
