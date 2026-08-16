source ../helpers.nu

# Test objdump -ht: section-headers immediately followed by SYMBOL TABLE (no blank between)
try { ^$env.REF -ht $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -ht $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "objdump -ht"
