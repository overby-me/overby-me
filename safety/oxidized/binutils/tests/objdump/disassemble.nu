source ../helpers.nu

# Test objdump -d (disassemble)
try { ^$env.REF -d $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -d $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "objdump -d (disassemble)"
