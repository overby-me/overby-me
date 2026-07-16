touch "$TMPDIR/nonsocket"
PIPEWIRE_REMOTE="$TMPDIR/nonsocket" "$REF"  i Core </dev/null > "$TMPDIR/expected" 2>&1 || true
PIPEWIRE_REMOTE="$TMPDIR/nonsocket" "$RUST" i Core </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli i Core (PIPEWIRE_REMOTE bad → connect error)"
