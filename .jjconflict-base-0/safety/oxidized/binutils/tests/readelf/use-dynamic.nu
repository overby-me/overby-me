source ../helpers.nu

# Test readelf --syms --use-dynamic on file without dynamic symbols
try { ^$env.REF --syms --use-dynamic $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --syms --use-dynamic $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf --syms --use-dynamic"
