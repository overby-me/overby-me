source ../helpers.nu

# Test objdump -t -j SECTION: symbol table filtered to one section
try { ^$env.REF -t -j .text $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -t -j .text $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "objdump -t -j .text"
