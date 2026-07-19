source ../helpers.nu

^$env.REF -i -l o+e> ($env.TMPDIR | path join expected)
^$env.RUST -i -l o+e> ($env.TMPDIR | path join actual)
compare "pw-link -i -l (rich daemon)"
