source ../helpers.nu

# Test ar tO: list members with their archive file offsets
let objs = $env.TMPDIR | path join objs
mkdir $objs
"int foo(int x){return x+1;}\n" | gcc -c -x c - -o ($objs | path join foo.o)
"int bar(int x){return x*2;}\n" | gcc -c -x c - -o ($objs | path join bar.o)
let archive = $objs | path join libtest.a
^$env.REF cr $archive ($objs | path join foo.o) ($objs | path join bar.o)

do { cd $objs; try { ^$env.REF tO libtest.a o+e> ($env.TMPDIR | path join expected) } }
do { cd $objs; try { ^$env.RUST tO libtest.a o+e> ($env.TMPDIR | path join actual) } }
compare "ar tO"
