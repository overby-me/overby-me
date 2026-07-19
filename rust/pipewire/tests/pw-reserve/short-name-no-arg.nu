source ../helpers.nu

# pw-reserve -n (short form of --name) requires an argument.
try { ^$env.REF -n o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -n o+e> ($env.TMPDIR | path join actual) }
compare "pw-reserve -n (short form requires arg)"
