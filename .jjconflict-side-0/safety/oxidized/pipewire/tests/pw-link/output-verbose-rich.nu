source ../helpers.nu

^$env.REF -o -v o+e> ($env.TMPDIR | path join expected)
^$env.RUST -o -v o+e> ($env.TMPDIR | path join actual)
compare "pw-link -o -v (rich daemon)"
