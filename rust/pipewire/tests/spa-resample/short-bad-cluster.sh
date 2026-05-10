"$REF"  -bx </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -bx </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample -bx (short bad cluster)"
