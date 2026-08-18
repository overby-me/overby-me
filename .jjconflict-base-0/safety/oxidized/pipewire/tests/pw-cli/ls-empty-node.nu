source ../helpers.nu

# In a stripped-down daemon (no audio backends, no monitors), there should
# be no Node globals: both binaries should print exactly nothing.
^$env.REF ls Node o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST ls Node o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-cli ls Node (none)"
