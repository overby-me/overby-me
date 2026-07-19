source ../helpers.nu

^$env.REF switch-remote o+e> ($env.TMPDIR | path join expected)
^$env.RUST switch-remote o+e> ($env.TMPDIR | path join actual)
compare "pw-cli switch-remote"
