source ../helpers.nu

# Test strings with two input files
let test2 = $env.TMPDIR | path join test2.o
cp $env.TESTOBJ $test2
try { ^$env.REF $env.TESTOBJ $test2 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST $env.TESTOBJ $test2 o+e> ($env.TMPDIR | path join actual) }
compare "strings multiple files"
