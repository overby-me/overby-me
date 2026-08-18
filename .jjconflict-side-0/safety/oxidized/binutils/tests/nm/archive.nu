source ../helpers.nu

# Test nm on an archive: should print "<member.o>:" header per member
let objs = $env.TMPDIR | path join objs
mkdir $objs
cp $env.TESTOBJ ($objs | path join bintest.o)
let archive = $objs | path join lib.a
ar cr $archive ($objs | path join bintest.o)

do { cd $objs; try { ^$env.REF lib.a o+e> ($env.TMPDIR | path join expected) } }
do { cd $objs; try { ^$env.RUST lib.a o+e> ($env.TMPDIR | path join actual) } }
compare "nm archive"
