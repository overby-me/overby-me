source ../helpers.nu

# pipewire-pulse -c (short --config) requires an argument.
try { ^$env.REF -c o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -c o+e> ($env.TMPDIR | path join actual) }
compare "pipewire-pulse -c (short form requires arg)"
