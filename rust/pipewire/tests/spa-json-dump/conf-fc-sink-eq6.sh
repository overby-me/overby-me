"$REF" "$SRC/src/daemon/filter-chain/sink-eq6.conf" > "$TMPDIR/expected" 2>&1
"$RUST" "$SRC/src/daemon/filter-chain/sink-eq6.conf" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/conf-fc-sink-eq6"
