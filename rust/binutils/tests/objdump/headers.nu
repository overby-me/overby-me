source ../helpers.nu

# Test objdump -h (section headers)
try { ^$env.REF -h $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -h $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "objdump -h (section headers)"
