source ../helpers.nu

# `pw-cli help` (the interactive command, not the --help flag) prints just
# the "Available commands:" list, no leading option summary.
^$env.REF help o+e> ($env.TMPDIR | path join expected)
^$env.RUST help o+e> ($env.TMPDIR | path join actual)
compare "pw-cli help (interactive command)"
