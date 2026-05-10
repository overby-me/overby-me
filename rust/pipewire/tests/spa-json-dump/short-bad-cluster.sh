"$REF"  -bx </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -bx </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-json-dump -bx (short bad cluster)"
