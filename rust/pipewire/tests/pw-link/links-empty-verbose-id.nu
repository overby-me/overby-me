source ../helpers.nu

# pw-link -l -I -v on a daemon with no links: produces no output
# (basic daemon).
^$env.REF -l -I -v o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST -l -I -v o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-link -l -I -v (basic daemon, empty)"
