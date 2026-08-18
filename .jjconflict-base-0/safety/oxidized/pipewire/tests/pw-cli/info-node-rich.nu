source ../helpers.nu

# Rich daemon: a null-audio-sink Node lives at id 8 (after Core/0,
# Module/1, SecurityContext/2, Module/3, Factory/4, Module/5, Metadata/6,
# Module/7-spa-node-factory, Node/8).
#
# Both binaries should bind the node and print the same info, params, props.
^$env.REF info 8 o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST info 8 o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-cli info 8 (Node, rich daemon)"
