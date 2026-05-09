unset PIPEWIRE_REMOTE
unset PIPEWIRE_CORE
"$REF"  -r /nonexistent </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -r /nonexistent </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-dump/connect-fail"
