source ../helpers.nu

# pw-link -o in the basic daemon: no nodes/ports means no output.
^$env.REF -o o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST -o o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-link -o (basic daemon, empty)"
