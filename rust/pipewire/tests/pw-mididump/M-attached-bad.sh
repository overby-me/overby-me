"$REF"  -Mh </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -Mh </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mididump -Mh (attached bad value)"
