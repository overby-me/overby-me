source ../helpers.nu

# Rich daemon: a null-audio-sink Node with 2 input ports (FL, FR).
# pw-link -i should list them in registry order; both binaries identical.
^$env.REF -i o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST -i o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-link -i (rich daemon, 2 input ports)"
