source ../helpers.nu

# Test readelf -p .comment: produces "section not dumped" warning when missing
try { ^$env.REF -p .comment $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -p .comment $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "readelf -p .comment (missing)"
