source ../helpers.nu

# Rich daemon: the null-audio-sink Node has no output ports.
^$env.REF -o o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST -o o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-link -o (rich daemon, no output ports)"
