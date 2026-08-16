source ../helpers.nu

# Test nm -S: BSD format showing symbol sizes
try { ^$env.REF -S $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -S $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm -S"
