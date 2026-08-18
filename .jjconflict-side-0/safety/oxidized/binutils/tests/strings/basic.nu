source ../helpers.nu

# Test strings on a binary file: should find printable strings
# Create a test file with embedded strings
let testfile = $env.TMPDIR | path join testfile
0x[67617262616765 000102 546869732069732061207465737420737472696e67 00
   6d6f726520676172626167 65 0001 416e6f7468657220737472696e672068657265 00]
| save -f --raw $testfile
try { ^$env.REF $testfile o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST $testfile o+e> ($env.TMPDIR | path join actual) }
compare "strings basic"
