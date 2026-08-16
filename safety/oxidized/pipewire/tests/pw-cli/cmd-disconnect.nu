source ../helpers.nu

^$env.REF disconnect o+e> ($env.TMPDIR | path join expected)
^$env.RUST disconnect o+e> ($env.TMPDIR | path join actual)
compare "pw-cli disconnect"
