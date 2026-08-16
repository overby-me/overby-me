source ../helpers.nu

# Real upstream config: src/daemon/pipewire-aes67.conf.in. Exercises the
# dotted-quad-as-string requoting (e.g. 239.255.255.255 -> "239.255.255.255").
let conf = ($env.SRC | path join src/daemon/pipewire-aes67.conf.in)
^$env.REF $conf o+e> ($env.TMPDIR | path join expected)
^$env.RUST $conf o+e> ($env.TMPDIR | path join actual)
compare "spa-json-dump/conf-aes67"
