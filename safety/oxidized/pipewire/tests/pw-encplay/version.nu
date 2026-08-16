source ../helpers.nu

# pw-encplay --version: byte-identical.
^$env.REF --version o+e> ($env.TMPDIR | path join expected)
^$env.RUST --version o+e> ($env.TMPDIR | path join actual)
compare "pw-encplay/version"
