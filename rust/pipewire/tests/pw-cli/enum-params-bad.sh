"$REF"  enum-params 0 0 </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" enum-params 0 0 </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli enum-params 0 0 (no daemon)"
