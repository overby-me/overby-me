# pw-config --name= (empty). Same code path as no-suffix — empty string
# doesn't end with .conf so we reject early.
"$REF"  --name= > "$TMPDIR/c.full" 2>&1 || true
e_ref=$?
"$RUST" --name= > "$TMPDIR/r.full" 2>&1 || true
e_rust=$?
sed -E 's|\[[0-9:.]+\]|[TIME]|' "$TMPDIR/c.full" > "$TMPDIR/expected"
sed -E 's|\[[0-9:.]+\]|[TIME]|' "$TMPDIR/r.full" > "$TMPDIR/actual"
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-config --name= (empty name rejected with 'does not end with .conf')"
