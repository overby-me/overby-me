source ../helpers.nu

# Test nm --no-weak: filter out weak symbols
try { ^$env.REF --no-weak $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --no-weak $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm --no-weak"
