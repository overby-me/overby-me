"$REF"  --rate=foo > "$TMPDIR/expected" 2>&1 || true
"$RUST" --rate=foo > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat --rate=foo (atoi → 0 → bad rate 0)"
