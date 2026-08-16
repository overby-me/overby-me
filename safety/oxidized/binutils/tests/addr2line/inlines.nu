source ../helpers.nu

# Test addr2line -i / --inlines: show inlined functions
try { ^$env.REF -i -e $env.TESTOBJ 0x0 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -i -e $env.TESTOBJ 0x0 o+e> ($env.TMPDIR | path join actual) }
compare "addr2line -i"
