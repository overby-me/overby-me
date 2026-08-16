source ../helpers.nu

# Rich daemon: a null-audio-sink Node has 2 input ports (FL, FR). ls Port
# should list them; both binaries should produce identical output.
^$env.REF ls Port o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST ls Port o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-cli ls Port (rich daemon)"
