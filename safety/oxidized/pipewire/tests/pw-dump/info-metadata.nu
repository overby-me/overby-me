source ../helpers.nu

# Rich daemon: the settings Metadata global lives at id 12 (registry
# layout: 0=Core, 1=SecurityContext, 2-3=Module, 4=Factory(ClientNode),
# 5-6=Module, 7=Factory(Node), 8=Module, 9=Factory(Metadata), 10=Node,
# 11=Port, 12=Metadata).
#
# pw-dump <metadata-id> should emit the registry props + the `"metadata"`
# items array (8 entries the daemon's metadata module pre-populates with
# clock.* / log.level defaults).
let cfull = $env.TMPDIR | path join c.full
let rfull = $env.TMPDIR | path join r.full
^$env.REF 12 o> $cfull e> ($env.TMPDIR | path join c.err)
^$env.RUST 12 o> $rfull e> ($env.TMPDIR | path join r.err)

cp $cfull ($env.TMPDIR | path join expected)
cp $rfull ($env.TMPDIR | path join actual)
compare "pw-dump 12 (Metadata items array: settings defaults, rich daemon)"
