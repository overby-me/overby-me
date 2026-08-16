source ../helpers.nu

# pipewire-pulse -P (short --properties) requires an argument.
try { ^$env.REF -P o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -P o+e> ($env.TMPDIR | path join actual) }
compare "pipewire-pulse -P (short form requires arg)"
