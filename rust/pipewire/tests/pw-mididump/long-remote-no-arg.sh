"$REF"  --remote </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --remote </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mididump --remote (long form requires arg)"
