source ../helpers.nu

# Test objdump -dt: symbol table followed by disassembly with correct blank-line spacing
try { ^$env.REF -dt $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -dt $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "objdump -dt"
