source ../helpers.nu

# Test ar: create an archive and list its contents
# Create a second object
cp $env.TESTOBJ ($env.TMPDIR | path join bintest2.o)
let ref_a = $env.TMPDIR | path join ref.a
let rust_a = $env.TMPDIR | path join rust.a
try { ^$env.REF cr $ref_a $env.TESTOBJ ($env.TMPDIR | path join bintest2.o) }
try { ^$env.RUST cr $rust_a $env.TESTOBJ ($env.TMPDIR | path join bintest2.o) }

# List contents of both archives using the same tool (reference) to compare archive format
try { ^$env.REF t $ref_a o+e> ($env.TMPDIR | path join expected) }
try { ^$env.REF t $rust_a o+e> ($env.TMPDIR | path join actual) }
compare "ar create and list"
