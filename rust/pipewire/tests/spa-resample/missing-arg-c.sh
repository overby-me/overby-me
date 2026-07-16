"$REF" -v -c </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -v -c </dev/null > "$TMPDIR/actual" 2>&1 || true
compare "spa-resample/missing-arg-c"
