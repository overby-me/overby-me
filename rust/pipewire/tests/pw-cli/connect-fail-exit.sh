PIPEWIRE_REMOTE=non-existent "$REF"  ls </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
PIPEWIRE_REMOTE=non-existent "$RUST" ls </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-cli ls connect-fail exits 255"
