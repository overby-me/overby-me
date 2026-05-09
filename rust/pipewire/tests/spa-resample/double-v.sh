"$REF"  -vv </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -vv </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample/double-v"
