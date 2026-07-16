PIPEWIRE_REMOTE=non-existent-socket "$REF"  help </dev/null > "$TMPDIR/expected" 2>&1 || true
PIPEWIRE_REMOTE=non-existent-socket "$RUST" help </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli with PIPEWIRE_REMOTE env (non-existent socket → 'Host is down')"
