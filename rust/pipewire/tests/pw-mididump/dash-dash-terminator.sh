"$REF"  -- /etc </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -- /etc </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mididump -- /etc (-- terminator + directory)"
