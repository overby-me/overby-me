"$REF"  --remote </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --remote </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-link --remote (long form requires arg)"
