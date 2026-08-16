source ../helpers.nu

# pw-cli create-node with no args prints `Error: "create-node <usage>"` to stderr.
try { ^$env.REF create-node o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST create-node o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli create-node (usage error)"
