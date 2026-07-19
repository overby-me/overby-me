source ../helpers.nu

# Test readelf -p .data: should print "No strings found in this section." for sections of all-non-printable bytes
try { ^$env.REF -p .data $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -p .data $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -p .data (no strings)"
