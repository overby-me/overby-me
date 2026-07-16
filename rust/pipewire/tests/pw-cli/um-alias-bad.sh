"$REF"  um foo bar </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" um foo bar </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli um foo bar (alias-aware)"
