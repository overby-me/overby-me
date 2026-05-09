"$REF"  card </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" card </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-acp-tool/card-no-args"
