source ../helpers.nu

^$env.REF --list-formats o+e> ($env.TMPDIR | path join expected)
^$env.RUST --list-formats o+e> ($env.TMPDIR | path join actual)
compare "pw-cat/list-formats"
