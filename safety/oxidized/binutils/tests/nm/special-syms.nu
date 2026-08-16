source ../helpers.nu

# Test nm --special-syms: accept the flag (no synthetic symbols produced)
try { ^$env.REF --special-syms $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --special-syms $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm --special-syms"
