source ../helpers.nu

# Test nm --reverse-sort --numeric-sort: combined sort flags
try { ^$env.REF --reverse-sort --numeric-sort $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --reverse-sort --numeric-sort $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm --reverse-sort --numeric-sort"
