"$REF" -h </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -h </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-monitor/help-flag"
