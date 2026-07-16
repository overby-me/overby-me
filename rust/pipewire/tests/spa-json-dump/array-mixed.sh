# Mixed-type array: numbers, strings, nested object, nested array.
cat > "$TMPDIR/in.json" <<'EOF'
arr = [ 1 "two" 3.14 { k = v } [ "nested" ] true false null ]
EOF
"$REF" "$TMPDIR/in.json" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.json" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/array-mixed"
