source ../helpers.nu

# Test nm -n / --numeric-sort: sort symbols by address
try { ^$env.REF -n $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -n $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm --numeric-sort"
