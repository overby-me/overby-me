source ../helpers.nu

# Test objdump -d -j SECTION: disassemble only the named section
try { ^$env.REF -d -j .data $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -d -j .data $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "objdump -d -j .data"
