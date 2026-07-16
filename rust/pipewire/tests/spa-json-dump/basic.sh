# Plain JSON object — exercise the round-trip through the parser/printer.
cat > "$TMPDIR/in.json" <<'EOF'
{ "a": 1, "b": "hello", "c": true, "d": null }
EOF
"$REF" "$TMPDIR/in.json" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.json" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/basic"
