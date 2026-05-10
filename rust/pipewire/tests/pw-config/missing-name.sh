"$REF"  -n nonexistent.conf paths </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" -n nonexistent.conf paths </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
sed -i -E 's/\[[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{6}\]/[TIME]/g; s/0x[0-9a-fA-F]+/0xPTR/g' "$TMPDIR/expected" "$TMPDIR/actual"
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-config -n nonexistent.conf (no prefix → 2-line log)"
