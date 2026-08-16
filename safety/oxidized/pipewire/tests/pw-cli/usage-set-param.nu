source ../helpers.nu

# pw-cli set-param with no args prints `Error: "set-param <usage>"` to stderr.
try { ^$env.REF set-param o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST set-param o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli set-param (usage error)"
