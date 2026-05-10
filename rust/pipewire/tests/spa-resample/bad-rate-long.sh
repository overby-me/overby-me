"$REF"  --rate=foo - - > "$TMPDIR/expected" 2>&1 || true
"$RUST" --rate=foo - - > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --rate=foo (non-numeric long form)"
