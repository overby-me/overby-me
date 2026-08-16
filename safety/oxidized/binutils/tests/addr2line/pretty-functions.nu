source ../helpers.nu

# Test addr2line -fp: pretty-print combines function and file:line on one line
try { ^$env.REF -fp -e $env.TESTOBJ 0x0 0x8 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -fp -e $env.TESTOBJ 0x0 0x8 o+e> ($env.TMPDIR | path join actual) }
compare "addr2line -fp"
