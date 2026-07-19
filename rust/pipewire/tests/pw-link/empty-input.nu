source ../helpers.nu

# pw-link -i in the basic daemon: no nodes/ports means no output.
^$env.REF -i o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST -i o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-link -i (basic daemon, empty)"
