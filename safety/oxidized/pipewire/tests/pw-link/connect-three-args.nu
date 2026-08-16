source ../helpers.nu

# C silently consumes only first 2 positional; extras are ignored. In
# the no-daemon sandbox both binaries emit "can't connect: Host is down".
try { ^$env.REF a b c o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST a b c o+e> ($env.TMPDIR | path join actual) }
compare "pw-link a b c (extra positional ignored)"
