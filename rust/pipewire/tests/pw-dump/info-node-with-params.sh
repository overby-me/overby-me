# Rich daemon: the configured null-audio-sink Node lives at id 10 (the
# fixed registry ordering in our test daemon: 0=Core, 1=SecurityContext,
# 2-3=Module, 4=Factory(ClientNode), 5-6=Module, 7=Factory(Node),
# 8=Module, 9=Factory(Metadata), 10=Node, 11=Port, 12=Metadata).
#
# pw-dump <node-id> should emit the full Node info block including the
# per-class params block: the Node's IO params (Clock, Position) as a
# JSON array under "IO" keyed by the SPA param short name.
"$REF"  10 </dev/null > "$TMPDIR/c.full"  2>"$TMPDIR/c.err"
"$RUST" 10 </dev/null > "$TMPDIR/r.full"  2>"$TMPDIR/r.err"

cp "$TMPDIR/c.full" "$TMPDIR/expected"
cp "$TMPDIR/r.full" "$TMPDIR/actual"
compare "pw-dump 10 (Node info block w/ IO params: Clock + Position, rich daemon)"
