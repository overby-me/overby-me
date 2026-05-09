"$REF"  /tmp/nonexistent1 /tmp/nonexistent2 </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" /tmp/nonexistent1 /tmp/nonexistent2 </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mididump foo bar (uses first positional)"
