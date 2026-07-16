"$REF" -V </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -V </dev/null > "$TMPDIR/actual" 2>&1 || true
compare "spa-json-dump/invalid-V"
