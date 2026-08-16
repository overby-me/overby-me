source ../helpers.nu

# Look up first global of type `Metadata` by substring match.
try { ^$env.REF info Metadata o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST info Metadata o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli info Metadata (by-name)"
