"$REF"  destroy 99 garbage </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" destroy 99 garbage </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli destroy 99 garbage (unknown global)"
