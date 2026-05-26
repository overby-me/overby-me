# Rich daemon: the null-audio-sink Node has 2 input ports (FL, FR) at
# the next two ids (9 and 10). pw-dump <port-id> should emit the
# per-class Port info block (direction, change-mask, props, params).
"$REF"  9 </dev/null > "$TMPDIR/c.full"  2>"$TMPDIR/c.err"
"$RUST" 9 </dev/null > "$TMPDIR/r.full"  2>"$TMPDIR/r.err"

cp "$TMPDIR/c.full" "$TMPDIR/expected"
cp "$TMPDIR/r.full" "$TMPDIR/actual"
compare "pw-dump 9 (Port info block: direction/change-mask/props/params, rich daemon)"
