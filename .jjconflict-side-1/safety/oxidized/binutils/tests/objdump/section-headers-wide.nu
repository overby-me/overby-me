source ../helpers.nu

# Test objdump -hw: wide section headers (single-line per section, with Flags)
try { ^$env.REF -hw $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -hw $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "objdump -hw"
