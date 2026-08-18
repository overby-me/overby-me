source ../helpers.nu

# Test readelf -h with two input files
let test2 = $env.TMPDIR | path join test2.o
cp $env.TESTOBJ $test2
try { ^$env.REF -h $env.TESTOBJ $test2 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -h $env.TESTOBJ $test2 o+e> ($env.TMPDIR | path join actual) }
compare "readelf -h multiple files"
