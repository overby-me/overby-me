source ../helpers.nu

# Rich daemon: pre-loaded with a null-audio-sink Node. ls Node should find
# at least one entry; both binaries should produce identical output.
^$env.REF ls Node o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST ls Node o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-cli ls Node (rich daemon)"
