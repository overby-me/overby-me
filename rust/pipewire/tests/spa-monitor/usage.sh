# spa-monitor with no args prints `usage: ... <plugin.so>`.
"$REF" </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-monitor/usage"
