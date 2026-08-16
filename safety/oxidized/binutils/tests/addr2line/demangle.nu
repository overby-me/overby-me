source ../helpers.nu

# Test addr2line -C: demangle function names
try { ^$env.REF -C -e $env.TESTOBJ 0x0 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -C -e $env.TESTOBJ 0x0 o+e> ($env.TMPDIR | path join actual) }
compare "addr2line -C"
