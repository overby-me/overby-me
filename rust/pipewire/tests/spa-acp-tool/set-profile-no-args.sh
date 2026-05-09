"$REF"  set-profile </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" set-profile </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-acp-tool/set-profile-no-args"
