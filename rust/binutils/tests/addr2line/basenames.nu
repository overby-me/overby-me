source ../helpers.nu

# Test addr2line --basenames: strip directory components
try { ^$env.REF --basenames -e $env.TESTOBJ 0x0 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --basenames -e $env.TESTOBJ 0x0 o+e> ($env.TMPDIR | path join actual) }
compare "addr2line --basenames"
