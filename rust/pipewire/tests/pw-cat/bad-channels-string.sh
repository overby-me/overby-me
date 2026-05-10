"$REF"  --channels=foo > "$TMPDIR/expected" 2>&1 || true
"$RUST" --channels=foo > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat --channels=foo (atoi → 0 → bad channels 0)"
