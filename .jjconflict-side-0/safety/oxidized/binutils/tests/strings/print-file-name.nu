source ../helpers.nu

# Test strings -f: prefix each string with the input file name
try { ^$env.REF -f $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -f $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "strings -f"
