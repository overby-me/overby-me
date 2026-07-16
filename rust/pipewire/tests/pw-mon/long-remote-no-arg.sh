"$REF"  --remote </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --remote </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mon --remote (long form requires arg)"
