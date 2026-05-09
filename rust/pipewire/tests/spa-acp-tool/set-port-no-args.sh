"$REF"  set-port </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" set-port </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-acp-tool/set-port-no-args"
