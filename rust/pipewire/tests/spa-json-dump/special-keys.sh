# Keys with dots, dashes, slashes, and quoted special chars.
cat > "$TMPDIR/in.json" <<'EOF'
"node.name" = "alice"
"node-with-dash" = 1
"foo/bar" = "slash"
"key with spaces" = ok
EOF
"$REF" "$TMPDIR/in.json" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.json" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/special-keys"
