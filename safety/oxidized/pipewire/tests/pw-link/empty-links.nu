source ../helpers.nu

# pw-link -l in the basic daemon: no links means no output.
^$env.REF -l o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST -l o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-link -l (basic daemon, empty)"
