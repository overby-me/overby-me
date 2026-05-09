touch "$TMPDIR/nonsocket"
PIPEWIRE_REMOTE="$TMPDIR/nonsocket" "$REF"  </dev/null > "$TMPDIR/expected" 2>&1 || true
PIPEWIRE_REMOTE="$TMPDIR/nonsocket" "$RUST" </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-top (Connection refused)"
