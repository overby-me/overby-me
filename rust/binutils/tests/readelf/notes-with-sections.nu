source ../helpers.nu

# Test readelf --notes -S: combined notes + section headers
try { ^$env.REF --notes -S $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --notes -S $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf --notes -S"
