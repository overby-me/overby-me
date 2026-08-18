source ../helpers.nu

# Rich daemon: the configured null-audio-sink Node lives at id 10 (the
# fixed registry ordering in our test daemon: 0=Core, 1=SecurityContext,
# 2-3=Module, 4=Factory(ClientNode), 5-6=Module, 7=Factory(Node),
# 8=Module, 9=Factory(Metadata), 10=Node, 11=Port, 12=Metadata).
#
# pw-dump <node-id> should emit the full Node info block including the
# per-class params block: the Node's IO params (Clock, Position) as a
# JSON array under "IO" keyed by the SPA param short name.
let cfull = $env.TMPDIR | path join c.full
let rfull = $env.TMPDIR | path join r.full
^$env.REF 10 o> $cfull e> ($env.TMPDIR | path join c.err)
^$env.RUST 10 o> $rfull e> ($env.TMPDIR | path join r.err)

cp $cfull ($env.TMPDIR | path join expected)
cp $rfull ($env.TMPDIR | path join actual)
compare "pw-dump 10 (Node info block w/ IO params: Clock + Position, rich daemon)"
