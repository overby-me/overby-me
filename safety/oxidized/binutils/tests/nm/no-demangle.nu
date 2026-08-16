source ../helpers.nu

# Test nm --no-demangle: accept the flag (no-op since we don't demangle by default)
try { ^$env.REF --no-demangle $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --no-demangle $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm --no-demangle"
