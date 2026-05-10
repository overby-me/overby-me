printf '0x10' | "$REF"  - > "$TMPDIR/expected" 2>&1 || true
printf '0x10' | "$RUST" - > "$TMPDIR/actual"   2>&1 || true
compare "spa-json-dump - <0x10> (bare number → empty object)"
