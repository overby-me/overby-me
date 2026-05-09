"$REF"  --recurse=foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --recurse=foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-config --recurse=foo (no-arg flag)"
