"$REF"  -R non-existent -p /etc/passwd </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" -R non-existent -p /etc/passwd </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-cat -R non-existent (explicit remote → connect-fail)"
