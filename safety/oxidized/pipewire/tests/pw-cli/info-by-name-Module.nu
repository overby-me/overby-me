source ../helpers.nu

# Look up first global of type `Module` by substring match.
try { ^$env.REF info Module o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST info Module o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli info Module (by-name)"
