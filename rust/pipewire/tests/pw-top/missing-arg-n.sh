"$REF" -n </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -n </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-top/missing-arg-n"
