# pipewire-pulse -c (short --config) requires an argument.
"$REF"  -c </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -c </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pipewire-pulse -c (short form requires arg)"
