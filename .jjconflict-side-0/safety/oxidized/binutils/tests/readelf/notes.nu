source ../helpers.nu

# Test readelf --notes / -n: display notes sections
try { ^$env.REF --notes $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --notes $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf --notes"
