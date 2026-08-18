source ../helpers.nu

# Test objdump -dM intel: bundled short opt with -M (takes argument)
try { ^$env.REF -dM intel $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -dM intel $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "objdump -dM intel"
