source ../helpers.nu

# Test readelf --histogram: bucket histogram for hash sections
try { ^$env.REF --histogram $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --histogram $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf --histogram"
