source ../helpers.nu

# Test objdump -f / --file-headers: display file header info
try { ^$env.REF -f $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -f $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "objdump -f"
