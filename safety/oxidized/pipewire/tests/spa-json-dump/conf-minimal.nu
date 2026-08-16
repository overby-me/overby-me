source ../helpers.nu

# Real upstream config: src/daemon/minimal.conf.in.
let conf = ($env.SRC | path join src/daemon/minimal.conf.in)
^$env.REF $conf o+e> ($env.TMPDIR | path join expected)
^$env.RUST $conf o+e> ($env.TMPDIR | path join actual)
compare "spa-json-dump/conf-minimal"
