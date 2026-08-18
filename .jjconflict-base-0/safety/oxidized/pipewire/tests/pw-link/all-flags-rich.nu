source ../helpers.nu

^$env.REF -i -o -l -I o+e> ($env.TMPDIR | path join expected)
^$env.RUST -i -o -l -I o+e> ($env.TMPDIR | path join actual)
compare "pw-link -i -o -l -I (rich daemon)"
