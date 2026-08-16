source ../helpers.nu

# Test readelf -l (program headers) on the test object
# Object files typically have no program headers, so output should indicate that
try { ^$env.REF -l $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -l $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -l (program headers)"
