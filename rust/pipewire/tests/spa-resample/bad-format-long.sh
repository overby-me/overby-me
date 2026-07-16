"$REF"  --format=invalid - - > "$TMPDIR/expected" 2>&1 || true
"$RUST" --format=invalid - - > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --format=invalid (invalid format long form)"
