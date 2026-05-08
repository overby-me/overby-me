# -s: output simplified SPA JSON (no commas, '=' separator, unquoted barewords).
cat > "$TMPDIR/in.json" <<'EOF'
{ "first": "value", "second": 42, "list": [ "a", "b", "c" ] }
EOF
"$REF" -s "$TMPDIR/in.json" > "$TMPDIR/expected" 2>&1
"$RUST" -s "$TMPDIR/in.json" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/simplified"
