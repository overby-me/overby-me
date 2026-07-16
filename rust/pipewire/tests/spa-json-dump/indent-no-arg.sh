"$REF"  --indent </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --indent </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-json-dump --indent (long form requires arg)"
