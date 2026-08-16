source ../helpers.nu

# Test strings -n (minimum length)
let testfile = $env.TMPDIR | path join testfile
0x[6162 00 61626364 00 6162636465666768 00] | save -f --raw $testfile
try { ^$env.REF -n 5 $testfile o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -n 5 $testfile o+e> ($env.TMPDIR | path join actual) }
compare "strings -n 5 (minimum length)"
