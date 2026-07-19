source ../helpers.nu

# Test addr2line -f: function-name resolution via symbol-table fallback
try { ^$env.REF -f -e $env.TESTOBJ 0x0 0x4 0x8 0xc o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -f -e $env.TESTOBJ 0x0 0x4 0x8 0xc o+e> ($env.TMPDIR | path join actual) }
compare "addr2line -f"
