# Rich daemon: the settings Metadata global lives at id 12 (registry
# layout: 0=Core, 1=SecurityContext, 2-3=Module, 4=Factory(ClientNode),
# 5-6=Module, 7=Factory(Node), 8=Module, 9=Factory(Metadata), 10=Node,
# 11=Port, 12=Metadata).
#
# pw-dump <metadata-id> should emit the registry props + the `"metadata"`
# items array (8 entries the daemon's metadata module pre-populates with
# clock.* / log.level defaults).
"$REF"  12 </dev/null > "$TMPDIR/c.full"  2>"$TMPDIR/c.err"
"$RUST" 12 </dev/null > "$TMPDIR/r.full"  2>"$TMPDIR/r.err"

cp "$TMPDIR/c.full" "$TMPDIR/expected"
cp "$TMPDIR/r.full" "$TMPDIR/actual"
compare "pw-dump 12 (Metadata items array — settings defaults, rich daemon)"
