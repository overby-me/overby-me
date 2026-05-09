"$REF"  i 0 garbage </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" i 0 garbage </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli i 0 garbage (multi-arg join semantics)"
