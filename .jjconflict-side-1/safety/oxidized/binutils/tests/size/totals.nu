source ../helpers.nu

# Test size -t (show totals) with multiple files
let copy = $env.TMPDIR | path join copy.o
cp $env.TESTOBJ $copy
try { ^$env.REF -t $env.TESTOBJ $copy o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -t $env.TESTOBJ $copy o+e> ($env.TMPDIR | path join actual) }
compare "size -t (totals)"
