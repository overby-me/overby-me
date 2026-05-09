"$REF"  -r=foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -r=foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli -r=foo (short with =value)"
