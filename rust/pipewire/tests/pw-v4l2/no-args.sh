"$REF" </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-v4l2 (no args silent exit)"
