# Deeply nested structure — verifies indent stacking.
cat > "$TMPDIR/in.json" <<'EOF'
outer = {
  middle = {
    inner = {
      list = [ { x = 1 } { x = 2 } ]
    }
  }
}
EOF
"$REF" "$TMPDIR/in.json" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.json" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/nested"
