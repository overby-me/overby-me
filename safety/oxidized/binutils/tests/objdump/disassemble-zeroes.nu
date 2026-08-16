source ../helpers.nu

# Test objdump -dz: disassemble zero bytes (no `\t...` collapse)
try { ^$env.REF -dz $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -dz $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "objdump -dz"
