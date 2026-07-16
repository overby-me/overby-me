"$REF" missing-file </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" missing-file </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
sed -i "s|0x[0-9a-fA-F]*|0xPTR|g" "$TMPDIR/expected" "$TMPDIR/actual"
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-encplay missing-file (avformat: No such file)"
