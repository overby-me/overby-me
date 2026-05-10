"$REF"  --bogus </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" --bogus </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
sed -i "s|/nix/store/[a-z0-9]\{32\}-[^/[:space:]]*|TOOL|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-top --bogus exits 255"
