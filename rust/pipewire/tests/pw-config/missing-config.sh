"$REF"  -p /etc -n bogus.conf paths </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" -p /etc -n bogus.conf paths </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
sed -i -E 's/\[[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{6}\]/[TIME]/g' "$TMPDIR/expected" "$TMPDIR/actual"
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-config -p /etc -n bogus.conf (missing config → exit 254)"
