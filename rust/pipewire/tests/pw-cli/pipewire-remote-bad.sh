# PIPEWIRE_REMOTE pointing to non-socket file: any command attempts
# connect on startup, errors with "Connection refused".
touch "$TMPDIR/nonsocket"
PIPEWIRE_REMOTE="$TMPDIR/nonsocket" "$REF"  help </dev/null > "$TMPDIR/expected" 2>&1 || true
PIPEWIRE_REMOTE="$TMPDIR/nonsocket" "$RUST" help </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli help with PIPEWIRE_REMOTE=non-socket"
