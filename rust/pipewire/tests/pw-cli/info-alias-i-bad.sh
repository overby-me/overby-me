"$REF"  i Audio </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" i Audio </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli i Audio (alias error format)"
