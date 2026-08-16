source ../helpers.nu

# Test addr2line on an object with no debug info: should output ??:0
try { "0x0\n" | ^$env.REF -e $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { "0x0\n" | ^$env.RUST -e $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "addr2line basic (no debug info)"
