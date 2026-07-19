source ../helpers.nu

# Look up first global of type `Client` by substring match.
try { ^$env.REF info Client o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST info Client o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli info Client (by-name)"
