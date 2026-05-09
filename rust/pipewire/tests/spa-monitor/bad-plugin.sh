"$REF" /nonexistent.so > "$TMPDIR/expected" 2>&1
"$RUST" /nonexistent.so > "$TMPDIR/actual" 2>&1
compare "spa-monitor/bad-plugin"
