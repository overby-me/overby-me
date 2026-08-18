source ../helpers.nu

^$env.REF --list-channel-names o+e> ($env.TMPDIR | path join expected)
^$env.RUST --list-channel-names o+e> ($env.TMPDIR | path join actual)
compare "pw-cat/list-channel-names"
