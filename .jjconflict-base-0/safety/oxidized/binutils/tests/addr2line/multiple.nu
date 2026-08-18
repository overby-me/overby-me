source ../helpers.nu

# Test addr2line with multiple addresses passed at once
try { ^$env.REF -e $env.TESTOBJ 0 4 8 c o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -e $env.TESTOBJ 0 4 8 c o+e> ($env.TMPDIR | path join actual) }
compare "addr2line multiple"
