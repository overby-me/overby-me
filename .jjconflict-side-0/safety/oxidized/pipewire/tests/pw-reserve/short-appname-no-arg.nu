source ../helpers.nu

# pw-reserve -a (short form of --appname) requires an argument.
try { ^$env.REF -a o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -a o+e> ($env.TMPDIR | path join actual) }
compare "pw-reserve -a (short form requires arg)"
