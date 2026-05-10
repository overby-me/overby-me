touch "$TMPDIR/nonsocket"
PIPEWIRE_REMOTE="$TMPDIR/nonsocket" "$REF"  quit </dev/null > "$TMPDIR/expected" 2>&1 || true
PIPEWIRE_REMOTE="$TMPDIR/nonsocket" "$RUST" quit </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli quit (PIPEWIRE_REMOTE bad → connect error)"
