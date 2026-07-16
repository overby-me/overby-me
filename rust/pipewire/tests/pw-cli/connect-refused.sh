# When PIPEWIRE_REMOTE points to a non-socket file, the connect fails
# with ECONNREFUSED instead of ENOENT — verify the wording.
touch "$TMPDIR/nonsocket"
PIPEWIRE_REMOTE="$TMPDIR/nonsocket" "$REF"  ls </dev/null > "$TMPDIR/expected" 2>&1 || true
PIPEWIRE_REMOTE="$TMPDIR/nonsocket" "$RUST" ls </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli ls (PIPEWIRE_REMOTE=non-socket → Connection refused)"
