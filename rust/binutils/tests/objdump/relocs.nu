source ../helpers.nu

# Test objdump -r (relocations)
try { ^$env.REF -r $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -r $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "objdump -r (relocations)"
