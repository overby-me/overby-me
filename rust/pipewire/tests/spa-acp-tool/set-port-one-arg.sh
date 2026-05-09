"$REF"  set-port 0 </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" set-port 0 </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-acp-tool/set-port-one-arg"
