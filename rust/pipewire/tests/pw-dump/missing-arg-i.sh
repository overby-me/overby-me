"$REF" -i </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -i </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-dump/missing-arg-i"
