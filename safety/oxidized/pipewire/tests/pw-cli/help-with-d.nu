source ../helpers.nu

^$env.REF -d help o+e> ($env.TMPDIR | path join expected)
^$env.RUST -d help o+e> ($env.TMPDIR | path join actual)
compare "pw-cli -d help"
