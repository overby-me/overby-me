"$REF"  -c </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -c </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-acp-tool/missing-arg-c"
