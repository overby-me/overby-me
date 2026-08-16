source ../helpers.nu

# Test size with no arguments (Berkeley format)
try { ^$env.REF $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "size basic (Berkeley format)"
