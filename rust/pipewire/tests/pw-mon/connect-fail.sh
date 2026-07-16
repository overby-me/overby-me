# Without a daemon, pw-mon should print can't connect: Host is down.
export XDG_RUNTIME_DIR="$TMPDIR/no-runtime"
"$REF"  </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mon (no daemon)"
