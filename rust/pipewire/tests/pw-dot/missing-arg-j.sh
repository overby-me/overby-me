"$REF" -j </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -j </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-dot/missing-arg-j"
