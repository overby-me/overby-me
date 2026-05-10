printf '# this is a comment\n# and another\n' | "$REF"  - > "$TMPDIR/expected" 2>&1 || true
printf '# this is a comment\n# and another\n' | "$RUST" - > "$TMPDIR/actual"   2>&1 || true
compare "spa-json-dump - <comment-only> ('not a valid file')"
