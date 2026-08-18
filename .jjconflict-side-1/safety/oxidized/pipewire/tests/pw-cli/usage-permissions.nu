source ../helpers.nu

# pw-cli permissions with no args prints `Error: "permissions <usage>"` to stderr.
try { ^$env.REF permissions o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST permissions o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli permissions (usage error)"
