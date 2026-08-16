source ../helpers.nu

# pw-midi2record --version: byte-identical.
^$env.REF --version o+e> ($env.TMPDIR | path join expected)
^$env.RUST --version o+e> ($env.TMPDIR | path join actual)
compare "pw-midi2record/version"
