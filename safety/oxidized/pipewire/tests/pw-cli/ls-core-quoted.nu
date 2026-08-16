source ../helpers.nu

# `pw-cli "ls Core"` (single quoted argument) parses the same as
# `pw-cli ls Core` because the C tool joins all positional args with
# spaces, then splits on whitespace.
^$env.REF "ls Core" o+e> ($env.TMPDIR | path join expected)
^$env.RUST "ls Core" o+e> ($env.TMPDIR | path join actual)
compare "pw-cli 'ls Core' (joined input)"
