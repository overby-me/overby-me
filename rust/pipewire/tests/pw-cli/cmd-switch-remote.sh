"$REF"  switch-remote </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" switch-remote </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-cli switch-remote"
