"$REF"  --props </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" --props </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-link --props (long form requires arg)"
