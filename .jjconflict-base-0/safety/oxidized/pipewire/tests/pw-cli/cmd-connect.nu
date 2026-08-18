source ../helpers.nu

^$env.REF connect o+e> ($env.TMPDIR | path join expected)
^$env.RUST connect o+e> ($env.TMPDIR | path join actual)
compare "pw-cli connect"
