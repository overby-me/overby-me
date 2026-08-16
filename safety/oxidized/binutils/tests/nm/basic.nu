source ../helpers.nu

# Test nm with no arguments: should list symbols from an object file
try { ^$env.REF $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm basic symbol listing"
