source ../helpers.nu

# Test objdump -t (symbol table)
try { ^$env.REF -t $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -t $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "objdump -t (symbols)"
