source ../helpers.nu

# pw-cli create-device with no args prints `Error: "create-device <usage>"` to stderr.
try { ^$env.REF create-device o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST create-device o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli create-device (usage error)"
