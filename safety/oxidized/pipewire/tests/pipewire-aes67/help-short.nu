source ../helpers.nu

^$env.REF -h o+e> ($env.TMPDIR | path join expected)
^$env.RUST -h o+e> ($env.TMPDIR | path join actual)
compare "pipewire-aes67/help-short"
