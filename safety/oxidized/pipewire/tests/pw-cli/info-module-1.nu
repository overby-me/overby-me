source ../helpers.nu

# Module id 1 is libpipewire-module-protocol-native (always present).
^$env.REF info 1 o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST info 1 o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-cli info 1 (Module: protocol-native)"
