PIPEWIRE_REMOTE=non-existent-socket "$REF"  a b </dev/null > "$TMPDIR/expected" 2>&1 || true
PIPEWIRE_REMOTE=non-existent-socket "$RUST" a b </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-link with PIPEWIRE_REMOTE env (non-existent socket → can't connect)"
