"$REF" -x </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -x </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-top/short-bad-flag"
