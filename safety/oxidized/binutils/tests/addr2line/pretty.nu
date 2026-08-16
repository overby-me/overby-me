source ../helpers.nu

# Test addr2line -p: pretty-print "function at file:line" instead of newline-separated
try { ^$env.REF -p -e $env.TESTOBJ 0x0 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -p -e $env.TESTOBJ 0x0 o+e> ($env.TMPDIR | path join actual) }
compare "addr2line -p (pretty)"
