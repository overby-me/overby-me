"$REF"  set-volume </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" set-volume </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-acp-tool/set-volume-no-args"
