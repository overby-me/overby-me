PIPEWIRE_REMOTE=non-existent-socket "$REF"  -p /etc/passwd </dev/null > "$TMPDIR/expected" 2>&1 || true
PIPEWIRE_REMOTE=non-existent-socket "$RUST" -p /etc/passwd </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat -p with PIPEWIRE_REMOTE env (non-existent socket → connect failed)"
