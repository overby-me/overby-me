"$REF"  - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-link - (lone dash positional)"
