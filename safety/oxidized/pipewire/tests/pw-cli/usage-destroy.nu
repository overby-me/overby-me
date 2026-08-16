source ../helpers.nu

# pw-cli destroy with no args prints `Error: "destroy <usage>"` to stderr.
try { ^$env.REF destroy o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST destroy o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli destroy (usage error)"
