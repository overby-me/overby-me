source ../helpers.nu

# Test nm -g (--extern-only): show only external symbols
try { ^$env.REF -g $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -g $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm --extern-only"
