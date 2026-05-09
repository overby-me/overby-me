"$REF" -r </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -r </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli/missing-arg-r"
