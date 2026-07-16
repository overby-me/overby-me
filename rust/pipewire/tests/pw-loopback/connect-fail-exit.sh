PIPEWIRE_REMOTE=non-existent "$REF"  > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
PIPEWIRE_REMOTE=non-existent "$RUST" > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
sed -i "s/pw-loopback-[0-9]*/pw-loopback-PID/g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-loopback connect-fail exits 255"
