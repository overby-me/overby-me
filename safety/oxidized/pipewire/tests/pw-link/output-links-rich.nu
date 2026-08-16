source ../helpers.nu

^$env.REF -o -l o+e> ($env.TMPDIR | path join expected)
^$env.RUST -o -l o+e> ($env.TMPDIR | path join actual)
compare "pw-link -o -l (rich daemon)"
