"$REF"  --rate=-5 > "$TMPDIR/expected" 2>&1 || true
"$RUST" --rate=-5 > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat --rate=-5 (negative rejected)"
