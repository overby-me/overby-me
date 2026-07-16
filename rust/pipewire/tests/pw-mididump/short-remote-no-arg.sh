"$REF"  -r </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -r </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mididump -r (short form requires arg)"
