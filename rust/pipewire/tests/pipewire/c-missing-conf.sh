"$REF"  -c missing.conf </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" -c missing.conf </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
sed -i -E 's/\[[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{6}\]/[TIME]/g' "$TMPDIR/expected" "$TMPDIR/actual"
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pipewire -c missing.conf (file not found → exit 254)"
