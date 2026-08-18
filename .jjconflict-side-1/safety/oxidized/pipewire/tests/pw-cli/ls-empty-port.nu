source ../helpers.nu

# No ports: daemon has no nodes loaded.
^$env.REF ls Port o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST ls Port o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-cli ls Port (none)"
