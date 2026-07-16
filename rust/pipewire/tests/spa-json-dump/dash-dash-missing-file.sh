"$REF"  -- /nonexistent_xyz </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -- /nonexistent_xyz </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-json-dump -- /nonexistent (-- terminator + missing file)"
