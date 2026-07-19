source ../helpers.nu

# Test nm --no-recurse-limit: accept the demangler knob as a no-op
try { ^$env.REF --no-recurse-limit $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --no-recurse-limit $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm --no-recurse-limit"
