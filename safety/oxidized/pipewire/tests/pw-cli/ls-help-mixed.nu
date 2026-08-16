source ../helpers.nu

# `pw-cli ls -h`: the C tool joins all args and parses; -h becomes
# part of the filter string.
^$env.REF ls -h o+e> ($env.TMPDIR | path join expected)
^$env.RUST ls -h o+e> ($env.TMPDIR | path join actual)
compare "pw-cli ls -h (mixed args)"
