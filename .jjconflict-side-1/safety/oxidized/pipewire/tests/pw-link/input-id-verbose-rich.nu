source ../helpers.nu

^$env.REF -i -I -v o+e> ($env.TMPDIR | path join expected)
^$env.RUST -i -I -v o+e> ($env.TMPDIR | path join actual)
compare "pw-link -i -I -v (rich daemon)"
