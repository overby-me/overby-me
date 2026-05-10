"$REF"  --rate=0 > "$TMPDIR/expected" 2>&1 || true
"$RUST" --rate=0 > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat --rate=0 (zero rejected)"
