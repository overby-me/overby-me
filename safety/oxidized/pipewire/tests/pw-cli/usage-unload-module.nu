source ../helpers.nu

# pw-cli unload-module with no args prints `Error: "unload-module <usage>"` to stderr.
try { ^$env.REF unload-module o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST unload-module o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli unload-module (usage error)"
