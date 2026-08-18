source ../helpers.nu

# Test addr2line with an out-of-section address: GNU prints `??:0`
# (vs. `??:?` for in-section addresses without DWARF info)
try { ^$env.REF -e $env.TESTOBJ 0x10000 999 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -e $env.TESTOBJ 0x10000 999 o+e> ($env.TMPDIR | path join actual) }
compare "addr2line out-of-section addresses"
