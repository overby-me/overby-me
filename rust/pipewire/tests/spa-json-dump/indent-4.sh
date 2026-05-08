# -i 4: change indent width.
cat > "$TMPDIR/in.json" <<'EOF'
{ a = 1, b = [ 2 3 ] }
EOF
"$REF" -i 4 "$TMPDIR/in.json" > "$TMPDIR/expected" 2>&1
"$RUST" -i 4 "$TMPDIR/in.json" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/indent-4"
