"$REF"  -rfoo - - > "$TMPDIR/expected" 2>&1 || true
"$RUST" -rfoo - - > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample -rfoo (non-numeric → 'bad rate foo')"
