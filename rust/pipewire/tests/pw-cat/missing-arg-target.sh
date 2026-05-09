"$REF" --target </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --target </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat/missing-arg-target"
