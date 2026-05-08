# Number formats: int, negative, float, scientific notation, dotted-quad
# (which is NOT a number and must be requoted as a string).
cat > "$TMPDIR/in.json" <<'EOF'
{
  i = 42
  ineg = -7
  f = 1.5
  fexp = 1.5e10
  ip = 192.168.1.1
}
EOF
"$REF" "$TMPDIR/in.json" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.json" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/numbers"
