source ../helpers.nu

# Test objdump -h -j SECTION: section-headers filtered to one section
try { ^$env.REF -h -j .text $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -h -j .text $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "objdump -h -j .text"
