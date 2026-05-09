echo "{ a = 1 b = two }" > "$TMPDIR/in.json"
"$REF"  - < "$TMPDIR/in.json" > "$TMPDIR/expected" 2>&1
"$RUST" - < "$TMPDIR/in.json" > "$TMPDIR/actual"   2>&1
compare "spa-json-dump - (stdin marker)"
