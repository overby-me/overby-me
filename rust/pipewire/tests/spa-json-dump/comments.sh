# Input with #-comments and inline #-comments after values.
cat > "$TMPDIR/in.json" <<'EOF'
# leading comment
key1 = 1   # trailing comment
key2 = 2
# blank-line comment
key3 = 3
EOF
"$REF" "$TMPDIR/in.json" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.json" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/comments"
