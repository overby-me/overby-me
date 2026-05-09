"$REF"  get-volume </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" get-volume </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-acp-tool/get-volume-no-args"
