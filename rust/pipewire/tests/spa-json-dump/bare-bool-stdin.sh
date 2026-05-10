printf 'true' | "$REF"  - > "$TMPDIR/expected" 2>&1 || true
printf 'true' | "$RUST" - > "$TMPDIR/actual"   2>&1 || true
compare "spa-json-dump - <true> (bare bool → empty object)"
