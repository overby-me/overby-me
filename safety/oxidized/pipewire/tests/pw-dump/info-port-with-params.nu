source ../helpers.nu

# Rich daemon: the FL input port lives at id 11 (registry layout:
# 0=Core, 1=SecurityContext, 2-3=Module, 4=Factory(ClientNode),
# 5-6=Module, 7=Factory(Node), 8=Module, 9=Factory(Metadata), 10=Node,
# 11=Port, 12=Metadata).
#
# pw-dump <port-id> should emit the full Port info block including
# EnumFormat (Object-Format POD with mediaType/mediaSubtype/format/rate/
# channels/position), Format (empty), IO (Buffers + AsyncBuffers), and
# Buffers (empty).
let cfull = $env.TMPDIR | path join c.full
let rfull = $env.TMPDIR | path join r.full
^$env.REF 11 o> $cfull e> ($env.TMPDIR | path join c.err)
^$env.RUST 11 o> $rfull e> ($env.TMPDIR | path join r.err)

cp $cfull ($env.TMPDIR | path join expected)
cp $rfull ($env.TMPDIR | path join actual)
compare "pw-dump 11 (Port info block w/ EnumFormat: Object-Format + Choice, rich daemon)"
