source ../helpers.nu

# pw-top --version: byte-identical to upstream after store-path normalization.
^$env.REF --version o+e> ($env.TMPDIR | path join expected)
^$env.RUST --version o+e> ($env.TMPDIR | path join actual)
compare "pw-top/version"
