PIPEWIRE_REMOTE=non-existent "$REF"  </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
PIPEWIRE_REMOTE=non-existent "$RUST" </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-top connect-fail exits 255"
