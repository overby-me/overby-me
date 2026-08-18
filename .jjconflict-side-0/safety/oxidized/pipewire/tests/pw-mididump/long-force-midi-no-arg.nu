source ../helpers.nu

# pw-mididump --force-midi (long form of -M) requires an argument.
try { ^$env.REF --force-midi o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --force-midi o+e> ($env.TMPDIR | path join actual) }
compare "pw-mididump --force-midi (long form requires arg)"
