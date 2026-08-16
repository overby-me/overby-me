source ../helpers.nu

# pw-reserve -p (short form of --priority) requires an argument.
try { ^$env.REF -p o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -p o+e> ($env.TMPDIR | path join actual) }
compare "pw-reserve -p (short form requires arg)"
