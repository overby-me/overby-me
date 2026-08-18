source ../helpers.nu

# Rich daemon: the null-audio-sink Node has 2 input ports (FL, FR) at
# the next two ids (9 and 10). pw-dump <port-id> should emit the
# per-class Port info block (direction, change-mask, props, params).
let cfull = $env.TMPDIR | path join c.full
let rfull = $env.TMPDIR | path join r.full
^$env.REF 9 o> $cfull e> ($env.TMPDIR | path join c.err)
^$env.RUST 9 o> $rfull e> ($env.TMPDIR | path join r.err)

cp $cfull ($env.TMPDIR | path join expected)
cp $rfull ($env.TMPDIR | path join actual)
compare "pw-dump 9 (Port info block: direction/change-mask/props/params, rich daemon)"
