printf '   \n  \t \n' | "$REF"  - > "$TMPDIR/expected" 2>&1 || true
printf '   \n  \t \n' | "$RUST" - > "$TMPDIR/actual"   2>&1 || true
compare "spa-json-dump - <whitespace-only> ('not a valid file')"
