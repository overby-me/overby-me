PIPEWIRE_RUNTIME_DIR="$TMPDIR/nonexistent" "$REF"  -p /etc/passwd </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
PIPEWIRE_RUNTIME_DIR="$TMPDIR/nonexistent" "$RUST" -p /etc/passwd </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-cat with PIPEWIRE_RUNTIME_DIR=nonexistent (connect-fail)"
