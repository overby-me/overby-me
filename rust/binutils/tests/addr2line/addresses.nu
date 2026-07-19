source ../helpers.nu

# Test addr2line -a: show address before each entry
try { ^$env.REF -a -e $env.TESTOBJ 0x0 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -a -e $env.TESTOBJ 0x0 o+e> ($env.TMPDIR | path join actual) }
compare "addr2line -a"
