source ../helpers.nu

^$env.REF -i -o -l -I -v o+e> ($env.TMPDIR | path join expected)
^$env.RUST -i -o -l -I -v o+e> ($env.TMPDIR | path join actual)
compare "pw-link -i -o -l -I -v (rich daemon, all flags + verbose)"
