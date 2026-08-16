source ../helpers.nu

# Look up first global of type `Factory` by substring match.
try { ^$env.REF info Factory o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST info Factory o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli info Factory (by-name)"
