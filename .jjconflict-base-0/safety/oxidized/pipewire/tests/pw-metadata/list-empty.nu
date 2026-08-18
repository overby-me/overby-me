source ../helpers.nu

# Basic daemon: pw-metadata --list against a daemon that loads
# libpipewire-module-metadata still has the daemon-side metadata
# instances; both binaries should produce the same list.
^$env.REF --list o> ($env.TMPDIR | path join expected) e> ($env.TMPDIR | path join "expected.err")
^$env.RUST --list o> ($env.TMPDIR | path join actual) e> ($env.TMPDIR | path join "actual.err")
compare "pw-metadata --list (basic daemon)"
