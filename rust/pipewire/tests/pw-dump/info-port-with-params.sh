# Rich daemon: the FL input port lives at id 11 (registry layout:
# 0=Core, 1=SecurityContext, 2-3=Module, 4=Factory(ClientNode),
# 5-6=Module, 7=Factory(Node), 8=Module, 9=Factory(Metadata), 10=Node,
# 11=Port, 12=Metadata).
#
# pw-dump <port-id> should emit the full Port info block including
# EnumFormat (Object-Format POD with mediaType/mediaSubtype/format/rate/
# channels/position), Format (empty), IO (Buffers + AsyncBuffers), and
# Buffers (empty).
"$REF"  11 </dev/null > "$TMPDIR/c.full"  2>"$TMPDIR/c.err"
"$RUST" 11 </dev/null > "$TMPDIR/r.full"  2>"$TMPDIR/r.err"

cp "$TMPDIR/c.full" "$TMPDIR/expected"
cp "$TMPDIR/r.full" "$TMPDIR/actual"
compare "pw-dump 11 (Port info block w/ EnumFormat: Object-Format + Choice, rich daemon)"
