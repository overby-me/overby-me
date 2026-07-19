source ../helpers.nu

# pw-loopback -l (latency) requires an argument; getopt errors.
try { ^$env.REF -l o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -l o+e> ($env.TMPDIR | path join actual) }
compare "pw-loopback -l (missing latency argument)"
