source ../helpers.nu

# Test objdump -t on an archive: should print "In archive ARCH:" then per-member tables
let objs = $env.TMPDIR | path join objs
mkdir $objs
cp $env.TESTOBJ ($objs | path join bintest.o)
let archive = $objs | path join lib.a
ar cr $archive ($objs | path join bintest.o)

do { cd $objs; try { ^$env.REF -t lib.a o+e> ($env.TMPDIR | path join expected) } }
do { cd $objs; try { ^$env.RUST -t lib.a o+e> ($env.TMPDIR | path join actual) } }
compare "objdump -t archive"
