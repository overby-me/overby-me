source ../helpers.nu

# pw-cli load-module with no args prints `Error: "load-module <usage>"` to stderr.
try { ^$env.REF load-module o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST load-module o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli load-module (usage error)"
