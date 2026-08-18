source ../helpers.nu

# Test nm -s on an archive: print archive index then member symbols
let objs = $env.TMPDIR | path join objs
mkdir $objs
cp $env.TESTOBJ ($objs | path join bintest.o)
let archive = $objs | path join lib.a
ar cr $archive ($objs | path join bintest.o)

do { cd $objs; try { ^$env.REF -s lib.a o+e> ($env.TMPDIR | path join expected) } }
do { cd $objs; try { ^$env.RUST -s lib.a o+e> ($env.TMPDIR | path join actual) } }
compare "nm -s archive"
