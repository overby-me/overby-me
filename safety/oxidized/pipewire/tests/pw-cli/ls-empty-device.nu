source ../helpers.nu

# No devices: daemon has no spa devices.
^$env.REF ls Device o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST ls Device o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-cli ls Device (none)"
