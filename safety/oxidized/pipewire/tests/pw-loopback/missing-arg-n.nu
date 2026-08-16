source ../helpers.nu

# pw-loopback -n (node name) requires an argument; getopt errors.
try { ^$env.REF -n o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -n o+e> ($env.TMPDIR | path join actual) }
compare "pw-loopback -n (missing node-name argument)"
