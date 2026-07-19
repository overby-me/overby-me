source ../helpers.nu

# Test nm -A / --print-file-name: prefix output with file name
try { ^$env.REF -A $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -A $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm --print-file-name"
