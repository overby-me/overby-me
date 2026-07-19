source ../helpers.nu

# Test nm -j / --just-symbols: print only symbol names
try { ^$env.REF -j $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -j $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm --just-symbols"
