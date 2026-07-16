"$REF" "$SRC/src/daemon/filter-chain/36-dcblock.conf" > "$TMPDIR/expected" 2>&1
"$RUST" "$SRC/src/daemon/filter-chain/36-dcblock.conf" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/conf-fc-36-dcblock"
