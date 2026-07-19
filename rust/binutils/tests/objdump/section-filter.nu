source ../helpers.nu

# Test objdump -j .text -d: disassemble only the .text section
try { ^$env.REF -j .text -d $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -j .text -d $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "objdump -j .text -d"
