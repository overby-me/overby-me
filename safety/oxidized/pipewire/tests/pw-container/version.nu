source ../helpers.nu

^$env.REF --version o+e> ($env.TMPDIR | path join expected)
^$env.RUST --version o+e> ($env.TMPDIR | path join actual)
compare "pw-container/version"
