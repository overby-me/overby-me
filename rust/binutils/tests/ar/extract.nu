source ../helpers.nu

# Test ar: create archive then extract a member
cp $env.TESTOBJ ($env.TMPDIR | path join first.o)
cp $env.TESTOBJ ($env.TMPDIR | path join second.o)
let archive = $env.TMPDIR | path join test.a
try { ^$env.RUST cr $archive ($env.TMPDIR | path join first.o) ($env.TMPDIR | path join second.o) }

# Extract using both tools and compare the extracted file
let ref_extract = $env.TMPDIR | path join ref_extract
let rust_extract = $env.TMPDIR | path join rust_extract
mkdir $ref_extract $rust_extract
do { cd $ref_extract; try { ^$env.REF x $archive } }
do { cd $rust_extract; try { ^$env.RUST x $archive } }

# Compare extracted files
let res = do {
    ^diff ($ref_extract | path join first.o) ($rust_extract | path join first.o)
    ^diff ($ref_extract | path join second.o) ($rust_extract | path join second.o)
} | complete
if $res.exit_code == 0 {
    print "PASS: ar extract"
} else {
    print $res.stdout
    print "FAIL: ar extract"
    exit 1
}
