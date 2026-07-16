# Empty object and empty array.
cat > "$TMPDIR/in.json" <<'EOF'
empty_obj = {}
empty_arr = []
EOF
"$REF" "$TMPDIR/in.json" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.json" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/empty-object"
