"$REF"  set-mute </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" set-mute </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-acp-tool/set-mute-no-args"
