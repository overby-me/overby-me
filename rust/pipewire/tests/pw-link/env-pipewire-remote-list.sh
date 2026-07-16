PIPEWIRE_REMOTE=non-existent-socket "$REF"  -l </dev/null > "$TMPDIR/expected" 2>&1 || true
PIPEWIRE_REMOTE=non-existent-socket "$RUST" -l </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-link -l with PIPEWIRE_REMOTE env (list mode → can't connect)"
