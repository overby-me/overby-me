"$REF" "$SRC/src/daemon/filter-chain.conf.in" > "$TMPDIR/expected" 2>&1
"$RUST" "$SRC/src/daemon/filter-chain.conf.in" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/conf-filter-chain"
