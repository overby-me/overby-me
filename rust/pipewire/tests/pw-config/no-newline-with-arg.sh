"$REF"  --no-newline=foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --no-newline=foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-config --no-newline=foo (no-arg flag)"
