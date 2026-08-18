source ../helpers.nu

# pw-cli enum-params with no args prints `Error: "enum-params <usage>"` to stderr.
try { ^$env.REF enum-params o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST enum-params o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli enum-params (usage error)"
