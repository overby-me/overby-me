source ../helpers.nu

# Test nm -r / --reverse-sort: reverse the sort order
try { ^$env.REF -r $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -r $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm --reverse-sort"
