printf '"hello"' | "$REF"  - > "$TMPDIR/expected" 2>&1 || true
printf '"hello"' | "$RUST" - > "$TMPDIR/actual"   2>&1 || true
compare "spa-json-dump - <\"hello\"> (bare string → empty object)"
