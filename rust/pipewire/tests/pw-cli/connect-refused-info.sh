touch "$TMPDIR/nonsocket"
PIPEWIRE_REMOTE="$TMPDIR/nonsocket" "$REF"  i 0 </dev/null > "$TMPDIR/expected" 2>&1 || true
PIPEWIRE_REMOTE="$TMPDIR/nonsocket" "$RUST" i 0 </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli i 0 (Connection refused)"
