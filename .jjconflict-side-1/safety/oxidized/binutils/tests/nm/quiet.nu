source ../helpers.nu

# Test nm --quiet: accept the flag (terse archive listing; no-op for single ELF)
try { ^$env.REF --quiet $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --quiet $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm --quiet"
