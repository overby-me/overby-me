source ../helpers.nu

# Test objdump --start-address: synthetic label uses <sym+0xoffset> form
try { ^$env.REF -d --start-address=0x4 $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -d --start-address=0x4 $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "objdump --start-address=0x4"
