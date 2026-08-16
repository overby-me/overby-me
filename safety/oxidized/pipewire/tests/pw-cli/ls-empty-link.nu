source ../helpers.nu

# No links should be present in our minimal daemon.
^$env.REF ls Link o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST ls Link o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-cli ls Link (none)"
