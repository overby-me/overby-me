"$REF"  -r non-existent </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" -r non-existent </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
sed -i "s/pw-loopback-[0-9]*/pw-loopback-PID/g" "$TMPDIR/expected" "$TMPDIR/actual"
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-loopback -r non-existent (explicit remote)"
