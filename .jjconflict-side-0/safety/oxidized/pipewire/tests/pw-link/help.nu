source ../helpers.nu

# pw-link --help: byte-identical to upstream after store-path normalization.
^$env.REF --help o+e> ($env.TMPDIR | path join expected)
^$env.RUST --help o+e> ($env.TMPDIR | path join actual)
compare "pw-link/help"
