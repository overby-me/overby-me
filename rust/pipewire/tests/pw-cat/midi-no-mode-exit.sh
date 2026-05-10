"$REF"  -m </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" -m </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-cat -m alone (no primary mode → exit 1)"
