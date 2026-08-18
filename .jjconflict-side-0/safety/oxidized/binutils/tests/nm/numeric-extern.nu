source ../helpers.nu

# Test nm -ng: bundled short opts: numeric sort + extern only
try { ^$env.REF -ng $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -ng $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "nm -ng"
