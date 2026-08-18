source ../helpers.nu

# `pw-cli quit` / `q` produces no stdout, no stderr, exits 0.
^$env.REF quit o+e> ($env.TMPDIR | path join expected)
^$env.RUST quit o+e> ($env.TMPDIR | path join actual)
compare "pw-cli quit (silent exit)"
