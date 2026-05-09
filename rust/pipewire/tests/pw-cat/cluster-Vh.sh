"$REF" -Vh </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -Vh </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat/cluster-Vh"
