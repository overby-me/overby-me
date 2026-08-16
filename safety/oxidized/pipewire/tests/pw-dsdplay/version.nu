source ../helpers.nu

# pw-dsdplay --version: byte-identical.
^$env.REF --version o+e> ($env.TMPDIR | path join expected)
^$env.RUST --version o+e> ($env.TMPDIR | path join actual)
compare "pw-dsdplay/version"
