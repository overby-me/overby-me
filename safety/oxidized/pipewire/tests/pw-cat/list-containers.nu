source ../helpers.nu

^$env.REF --list-containers o+e> ($env.TMPDIR | path join expected)
^$env.RUST --list-containers o+e> ($env.TMPDIR | path join actual)
compare "pw-cat/list-containers"
