source ../helpers.nu

# Module id 3 is libpipewire-module-client-node in our test daemon.
^$env.REF info 3 o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST info 3 o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-cli info 3 (Module: client-node)"
