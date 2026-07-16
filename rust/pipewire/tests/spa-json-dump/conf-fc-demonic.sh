"$REF" "$SRC/src/daemon/filter-chain/demonic.conf" > "$TMPDIR/expected" 2>&1
"$RUST" "$SRC/src/daemon/filter-chain/demonic.conf" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/conf-fc-demonic"
