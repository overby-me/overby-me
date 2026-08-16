source ../helpers.nu

# Test nm -l / --line-numbers: annotate symbols with file:line (no-op
# for objects without DWARF)
try { ^$env.REF -l $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -l $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm -l"
