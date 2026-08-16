source ../helpers.nu

# Test nm -p (--no-sort): display symbols in order encountered
try { ^$env.REF -p $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -p $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm --no-sort"
