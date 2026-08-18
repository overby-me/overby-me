source ../helpers.nu

# Test nm -U / --defined-only: show only symbols with addresses
try { ^$env.REF --defined-only $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --defined-only $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm --defined-only"
