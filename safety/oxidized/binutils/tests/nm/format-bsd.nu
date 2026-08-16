source ../helpers.nu

# Test nm -B / --format=bsd: explicit BSD format (default)
try { ^$env.REF -B $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -B $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm -B"
