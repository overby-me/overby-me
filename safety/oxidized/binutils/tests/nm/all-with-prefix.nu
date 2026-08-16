source ../helpers.nu

# Test nm -aA: debug-syms (no-op) with file-name prefix
try { ^$env.REF -aA $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -aA $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm -aA"
