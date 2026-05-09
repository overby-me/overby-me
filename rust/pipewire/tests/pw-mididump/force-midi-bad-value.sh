"$REF"  --force-midi=foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --force-midi=foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mididump --force-midi=foo (bad value)"
