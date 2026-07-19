source ../helpers.nu

# Test strings on an object file
try { ^$env.REF $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "strings on object file"
