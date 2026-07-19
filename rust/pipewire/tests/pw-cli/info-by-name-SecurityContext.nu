source ../helpers.nu

# Look up first global of type `SecurityContext` by substring match.
try { ^$env.REF info SecurityContext o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST info SecurityContext o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli info SecurityContext (by-name)"
