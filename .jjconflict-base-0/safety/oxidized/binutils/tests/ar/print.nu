source ../helpers.nu

# Test ar -p: print contents of archive members to stdout
cp $env.TESTOBJ ($env.TMPDIR | path join m.o)
let archive = $env.TMPDIR | path join ar.a
try { ^$env.REF cr $archive ($env.TMPDIR | path join m.o) }
try { ^$env.REF -p $archive o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -p $archive o+e> ($env.TMPDIR | path join actual) }
compare "ar -p (print archive contents)"
