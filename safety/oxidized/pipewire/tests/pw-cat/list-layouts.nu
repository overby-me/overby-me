source ../helpers.nu

^$env.REF --list-layouts o+e> ($env.TMPDIR | path join expected)
^$env.RUST --list-layouts o+e> ($env.TMPDIR | path join actual)
compare "pw-cat/list-layouts"
