source ../helpers.nu

# Test objdump -dj.text: bundled short opt with -j (takes argument, no separator)
try { ^$env.REF -dj.text $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -dj.text $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "objdump -dj.text"
