source ../helpers.nu

# Test readelf -d / --dynamic: display dynamic section (or "no dynamic" message for .o)
try { ^$env.REF -d $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -d $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -d (dynamic section)"
