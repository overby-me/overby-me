"$REF" "$SRC/src/daemon/filter-chain/35-ebur128.conf" > "$TMPDIR/expected" 2>&1
"$RUST" "$SRC/src/daemon/filter-chain/35-ebur128.conf" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/conf-fc-35-ebur128"
