source ../helpers.nu

# Test objdump -f with two input files
let test2 = $env.TMPDIR | path join test2.o
cp $env.TESTOBJ $test2
try { ^$env.REF -f $env.TESTOBJ $test2 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -f $env.TESTOBJ $test2 o+e> ($env.TMPDIR | path join actual) }
compare "objdump -f multiple files"
