source ../helpers.nu

# pw-play --version: byte-identical.
^$env.REF --version o+e> ($env.TMPDIR | path join expected)
^$env.RUST --version o+e> ($env.TMPDIR | path join actual)
compare "pw-play/version"
