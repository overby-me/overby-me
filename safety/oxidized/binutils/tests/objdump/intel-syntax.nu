source ../helpers.nu

# Test objdump -d -M intel: Intel-syntax disassembly with size hints
try { ^$env.REF -d -M intel $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -d -M intel $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "objdump -d -M intel"
