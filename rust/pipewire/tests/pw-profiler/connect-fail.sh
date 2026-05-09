export XDG_RUNTIME_DIR="$TMPDIR/no-runtime"
"$REF"  </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-profiler (no daemon)"
