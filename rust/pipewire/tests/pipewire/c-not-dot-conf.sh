"$REF"  -c foo </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" -c foo </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
# Normalize timestamps in C output.
sed -i -E 's/\[[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{6}\]/[TIME]/g' "$TMPDIR/expected" "$TMPDIR/actual"
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pipewire -c foo (no .conf suffix → exit 234)"
