# spa-json-dump on a non-existent file: matches C's
# `error opening file 'X': <reason>` and exit 1.
"$REF"  /nonexistent.json > "$TMPDIR/expected" 2>&1 || true
"$RUST" /nonexistent.json > "$TMPDIR/actual"   2>&1 || true
compare "spa-json-dump/bad-file"
