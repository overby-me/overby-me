# Real upstream config: src/daemon/client.conf.in.
"$REF" "$SRC/src/daemon/client.conf.in" > "$TMPDIR/expected" 2>&1
"$RUST" "$SRC/src/daemon/client.conf.in" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/conf-client"
