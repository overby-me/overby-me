source ../helpers.nu

# Rich daemon: pw-metadata --list should find at least the "settings"
# metadata loaded by libpipewire-module-metadata.
^$env.REF --list o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join "expected.err")
^$env.RUST --list o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join "actual.err")
compare "pw-metadata --list (rich daemon)"
