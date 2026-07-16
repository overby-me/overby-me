cat > "$TMPDIR/in.json" <<'INNER'
a = { b = { c = { d = { e = { f = "deep" } } } } }
INNER
"$REF"  "$TMPDIR/in.json" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.json" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/deep-nesting"
