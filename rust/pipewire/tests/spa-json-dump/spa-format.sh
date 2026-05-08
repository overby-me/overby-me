# SPA-relaxed JSON: unquoted keys, '=' instead of ':', whitespace separators.
cat > "$TMPDIR/in.json" <<'EOF'
key1 = value1
key2 = "quoted value"
list = [ 1 2 3 ]
nested = { a = 1 b = 2 }
EOF
"$REF" "$TMPDIR/in.json" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.json" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/spa-format"
