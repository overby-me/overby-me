"$REF"  -hv </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -hv </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-acp-tool -hv (cluster)"
