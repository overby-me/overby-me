source ../helpers.nu

# Test addr2line -afp: addresses + functions + pretty-print combined
try { ^$env.REF -afp -e $env.TESTOBJ 0x0 0x8 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -afp -e $env.TESTOBJ 0x0 0x8 o+e> ($env.TMPDIR | path join actual) }
compare "addr2line -afp"
