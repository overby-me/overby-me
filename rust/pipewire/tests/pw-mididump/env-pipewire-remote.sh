PIPEWIRE_REMOTE=non-existent-socket "$REF"  </dev/null > "$TMPDIR/expected" 2>&1 || true
PIPEWIRE_REMOTE=non-existent-socket "$RUST" </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mididump with PIPEWIRE_REMOTE env (non-existent socket → can't connect)"
