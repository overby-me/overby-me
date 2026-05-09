"$REF"  ls - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" ls - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli ls - (lone dash positional)"
