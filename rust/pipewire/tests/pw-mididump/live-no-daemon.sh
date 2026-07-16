export XDG_RUNTIME_DIR="$TMPDIR/no-runtime"
unset PIPEWIRE_REMOTE
"$REF"  </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mididump (no daemon → can't connect)"
