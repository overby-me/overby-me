source ../helpers.nu

# Test nm -u (--undefined-only): show only undefined symbols
try { ^$env.REF -u $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -u $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm --undefined-only"
