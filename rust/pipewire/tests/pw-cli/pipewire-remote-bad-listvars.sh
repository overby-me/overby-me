touch "$TMPDIR/nonsocket"
PIPEWIRE_REMOTE="$TMPDIR/nonsocket" "$REF"  list-vars </dev/null > "$TMPDIR/expected" 2>&1 || true
PIPEWIRE_REMOTE="$TMPDIR/nonsocket" "$RUST" list-vars </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli list-vars (PIPEWIRE_REMOTE bad → connect error)"
