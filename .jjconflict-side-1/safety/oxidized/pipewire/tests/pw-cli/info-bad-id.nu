source ../helpers.nu

# Looking up an id that doesn't exist should print
# `Error: "info: unknown global 'X'"` to stderr (matching the C tool).
# Both binaries exit non-zero; we don't care about exit code, just output.
try { ^$env.REF info 9999 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST info 9999 o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli info 9999 (unknown global)"
