source ../helpers.nu

# Test nm -a / --debug-syms: include debug symbols (no-op for files without)
try { ^$env.REF -a $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -a $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm -a"
