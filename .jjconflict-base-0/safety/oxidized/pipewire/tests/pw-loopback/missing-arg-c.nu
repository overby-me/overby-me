source ../helpers.nu

# pw-loopback -c (channels) requires an argument; getopt errors.
try { ^$env.REF -c o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -c o+e> ($env.TMPDIR | path join actual) }
compare "pw-loopback -c (missing channels argument)"
