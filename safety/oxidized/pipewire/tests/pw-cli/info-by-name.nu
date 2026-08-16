source ../helpers.nu

# `pw-cli info <name>` looks up the first global whose type contains
# `<name>`. This mirrors the C tool's `find_global` substring fallback
# when the input isn't numeric.
^$env.REF info Core o+e> ($env.TMPDIR | path join expected)
^$env.RUST info Core o+e> ($env.TMPDIR | path join actual)
compare "pw-cli info Core (by-name lookup)"
