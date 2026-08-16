source ../helpers.nu

# Unknown subcommand: print the C tool's exact `Error: "Command ..."`
# message to stderr and exit 1.
try { ^$env.REF unknowncmd o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST unknowncmd o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli unknowncmd"
