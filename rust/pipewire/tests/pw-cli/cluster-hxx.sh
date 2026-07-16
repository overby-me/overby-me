"$REF"  -hxx </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -hxx </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli -hxx (cluster, h-first)"
