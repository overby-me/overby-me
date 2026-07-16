cat > "$TMPDIR/in.json" <<'INNER'
a = true
b = false
c = null
arr = [ true false null ]
INNER
"$REF"  "$TMPDIR/in.json" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.json" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/booleans"
