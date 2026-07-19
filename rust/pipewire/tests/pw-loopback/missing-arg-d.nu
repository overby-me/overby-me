source ../helpers.nu

# pw-loopback -d (delay) requires an argument; getopt errors.
try { ^$env.REF -d o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -d o+e> ($env.TMPDIR | path join actual) }
compare "pw-loopback -d (missing delay argument)"
