"$REF"  --raw -p missing-foo </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" --raw -p missing-foo </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-cat --raw -p missing-foo (raw mode → 'raw: can't open file')"
