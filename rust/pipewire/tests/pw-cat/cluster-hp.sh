"$REF" -hp </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -hp </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat/cluster-hp"
