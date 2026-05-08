# Quoted strings with escape sequences must round-trip verbatim.
cat > "$TMPDIR/in.json" <<'EOF'
{
  plain = "hello"
  esc = "line1\nline2"
  quote = "she said \"hi\""
  bslash = "C:\\path"
}
EOF
"$REF" "$TMPDIR/in.json" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.json" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/strings"
