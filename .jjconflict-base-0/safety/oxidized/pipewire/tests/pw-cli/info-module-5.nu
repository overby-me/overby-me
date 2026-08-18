source ../helpers.nu

# Module id 5 is libpipewire-module-access in our test daemon.
^$env.REF info 5 o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join expected.err)
^$env.RUST info 5 o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join actual.err)
compare "pw-cli info 5 (Module: access)"
